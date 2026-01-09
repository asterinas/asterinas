// SPDX-License-Identifier: MPL-2.0

use alloc::{borrow::ToOwned, collections::btree_set::BTreeSet, string::String, vec::Vec};
use core::{arch::x86_64::CpuidResult, ffi::CStr, fmt, str};

use ostd::{
    Pod,
    arch::{
        cpu::{
            context::{CpuException, PageFaultErrorCode, RawPageFaultInfo, UserContext},
            cpuid::cpuid,
        },
        tsc_freq,
    },
    cpu::{PinCurrentCpu, num_cpus},
    mm::Vaddr,
    sync::SpinLock,
    task::DisabledPreemptGuard,
};

use crate::{cpu::LinuxAbi, thread::exception::PageFaultInfo, vm::perms::VmPerms};

impl LinuxAbi for UserContext {
    fn syscall_num(&self) -> usize {
        self.rax()
    }

    fn syscall_ret(&self) -> usize {
        self.rax()
    }

    fn set_syscall_ret(&mut self, ret: usize) {
        self.set_rax(ret);
    }

    fn syscall_args(&self) -> [usize; 6] {
        [
            self.rdi(),
            self.rsi(),
            self.rdx(),
            self.r10(),
            self.r8(),
            self.r9(),
        ]
    }
}

/// Represents the context of a signal handler.
///
/// This contains the context saved before a signal handler is invoked; it will be restored by
/// `sys_rt_sigreturn`.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.15.7/source/arch/x86/include/uapi/asm/sigcontext.h#L325>
#[derive(Clone, Copy, Debug, Default, Pod)]
#[repr(C)]
pub struct SigContext {
    r8: usize,
    r9: usize,
    r10: usize,
    r11: usize,
    r12: usize,
    r13: usize,
    r14: usize,
    r15: usize,
    rdi: usize,
    rsi: usize,
    rbp: usize,
    rbx: usize,
    rdx: usize,
    rax: usize,
    rcx: usize,
    rsp: usize,
    rip: usize,
    rflags: usize,
    cs: u16,
    gs: u16,
    fs: u16,
    ss: u16,
    error_code: usize,
    trap_num: usize,
    old_mask: u64,
    page_fault_addr: usize,
    // A stack pointer to FPU context.
    fpu_context_addr: Vaddr,
    reserved: [u64; 8],
}

macro_rules! copy_gp_regs {
    ($src: ident, $dst: ident) => {
        $dst.rax = $src.rax;
        $dst.rbx = $src.rbx;
        $dst.rcx = $src.rcx;
        $dst.rdx = $src.rdx;
        $dst.rsi = $src.rsi;
        $dst.rdi = $src.rdi;
        $dst.rbp = $src.rbp;
        $dst.rsp = $src.rsp;
        $dst.r8 = $src.r8;
        $dst.r9 = $src.r9;
        $dst.r10 = $src.r10;
        $dst.r11 = $src.r11;
        $dst.r12 = $src.r12;
        $dst.r13 = $src.r13;
        $dst.r14 = $src.r14;
        $dst.r15 = $src.r15;
        $dst.rip = $src.rip;
        $dst.rflags = $src.rflags;
    };
}

impl SigContext {
    pub fn copy_user_regs_to(&self, dst: &mut UserContext) {
        let gp_regs = dst.general_regs_mut();
        copy_gp_regs!(self, gp_regs);
    }

    pub fn copy_user_regs_from(&mut self, src: &UserContext) {
        let gp_regs = src.general_regs();
        copy_gp_regs!(gp_regs, self);

        // TODO: Fill exception information in `SigContext`.
    }

    pub fn fpu_context_addr(&self) -> Vaddr {
        self.fpu_context_addr
    }

    pub fn set_fpu_context_addr(&mut self, addr: Vaddr) {
        self.fpu_context_addr = addr;
    }
}

impl From<&RawPageFaultInfo> for PageFaultInfo {
    fn from(raw_info: &RawPageFaultInfo) -> Self {
        let required_perms = if raw_info
            .error_code
            .contains(PageFaultErrorCode::INSTRUCTION)
        {
            VmPerms::EXEC
        } else if raw_info.error_code.contains(PageFaultErrorCode::WRITE) {
            // On x86_64, writable pages must also be readable.
            // Reference: Section 5.11.3 from <https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-3a-part-1-manual.pdf>.
            VmPerms::READ | VmPerms::WRITE
        } else {
            VmPerms::READ
        };

        PageFaultInfo::new(raw_info.addr, required_perms)
    }
}

impl TryFrom<&CpuException> for PageFaultInfo {
    // [`Err`] indicates that the [`CpuExceptionInfo`] is not a page fault,
    // with no additional error information.
    type Error = ();

    fn try_from(value: &CpuException) -> Result<Self, ()> {
        let CpuException::PageFault(raw_info) = value else {
            return Err(());
        };

        Ok(raw_info.into())
    }
}

#[derive(Clone, Copy)]
enum CpuVendor {
    Intel,
    Amd,
    Unknown,
}

impl CpuVendor {
    pub(self) fn parse(s: &str) -> Self {
        match s {
            "GenuineIntel" => Self::Intel,
            "AuthenticAMD" => Self::Amd,
            _ => Self::Unknown,
        }
    }
}

#[repr(u32)]
enum CpuidLeaf {
    Base = 0x00,
    Feature = 0x01,
    Cache = 0x04,
    Topology = 0x0b,

    // ExtBase = 0x80000000,
    ExtBrand1 = 0x80000002,
    ExtBrand2 = 0x80000003,
    ExtBrand3 = 0x80000004,
    ExtCacheL1 = 0x80000005,
    ExtCacheL2L3 = 0x80000006,
    ExtAddrSizes = 0x80000008,
}

/// A collection of CPU cores.
///
/// Note that hyperthreading can result in multiple logical processors within one core.
static CPU_CORES: SpinLock<BTreeSet<u32>> = SpinLock::new(BTreeSet::new());

/// CPU information to be shown in `/proc/cpuinfo`.
///
/// Different CPUs may have different information, such as the core ID. Therefore, [`Self::new`]
/// should be called on every CPU.
//
// Implementation notes: For each field in this structure that is conditionally shown in Linux, it
// should be wrapped in an `Option`. Please conduct the Linux implementation when adding a new
// field, see <https://elixir.bootlin.com/linux/v6.16/source/arch/x86/kernel/cpu/proc.c#L63>.
pub struct CpuInformation {
    processor: u32,
    vendor_id: String,
    cpu_family: u8,
    model: u8,
    model_name: String,
    stepping: Option<u8>,
    cpu_khz: Option<u32>,
    cache_size: Option<u32>,
    core_id: u32,
    apicid: u32,
    cpuid_level: u32,
    flags: Vec<&'static str>,
    tlb_size: Option<u32>,
    clflush_size: u32,
    cache_alignment: u32,
    address_sizes: (u32, u32),
}

impl CpuInformation {
    /// Constructs the information for the current CPU.
    pub fn new(guard: &DisabledPreemptGuard) -> Self {
        let mut result = Self {
            processor: guard.current_cpu().into(),
            vendor_id: "unknown".to_owned(),
            cpu_family: 0,
            model: 0,
            model_name: "unknown".to_owned(),
            stepping: None,
            // FIXME: The CPU frequency may not be equal to the TSC frequency. It may not even be a
            // constant (i.e., the real CPU frequency can be adjusted due to the workload).
            cpu_khz: Some((tsc_freq() / 1000) as u32),
            cache_size: None,
            core_id: 0,
            apicid: 0,
            cpuid_level: 0,
            flags: Vec::new(),
            tlb_size: None,
            clflush_size: 64,
            cache_alignment: 64,
            address_sizes: (36, 48),
        };

        let vendor = result.fill_vendor_info();
        result.fill_version_info(vendor);
        result.fill_brand_info();
        result.fill_cache_info(vendor);
        result.fill_topology_info();
        result.fill_feature_info();
        result.fill_address_info();

        result
    }

    fn fill_vendor_info(&mut self) -> CpuVendor {
        let Some(CpuidResult { eax, ebx, ecx, edx }) = cpuid(CpuidLeaf::Base as u32, 0) else {
            return CpuVendor::Unknown;
        };

        self.cpuid_level = eax;

        let vendor = [ebx.to_le_bytes(), edx.to_le_bytes(), ecx.to_le_bytes()];
        let Ok(vendor_str) = str::from_utf8(vendor.as_flattened()) else {
            return CpuVendor::Unknown;
        };
        self.vendor_id = vendor_str.to_owned();

        CpuVendor::parse(vendor_str)
    }

    fn fill_version_info(&mut self, vendor: CpuVendor) {
        let Some(CpuidResult { eax, ebx, .. }) = cpuid(CpuidLeaf::Feature as u32, 0) else {
            return;
        };

        let stepping = eax & 0xf;
        let base_model = (eax >> 4) & 0xf;
        let base_family = (eax >> 8) & 0xf;
        let ext_model = (eax >> 16) & 0xf;
        let ext_family = (eax >> 20) & 0xff;

        // "The Extended Family ID needs to be examined only when the Family ID is 0FH."
        let family = if base_family == 0xf {
            base_family + ext_family
        } else {
            base_family
        };

        let has_ext_model = match vendor {
            // The Intel manual says: "The Extended Model ID needs to be examined only when the
            // Family ID is 06H or 0FH."
            CpuVendor::Intel => base_family == 0x6 || base_family == 0xf,
            // The AMD manual says: "If BaseFamily[3:0] is less than Fh, then ExtFamily is reserved
            // and Family is equal to BaseFamily[3:0]."
            CpuVendor::Amd | CpuVendor::Unknown => base_family == 0xf,
        };
        let model = if has_ext_model {
            base_model | (ext_model << 4)
        } else {
            base_model
        };

        self.cpu_family = family as u8;
        self.model = model as u8;
        self.stepping = Some(stepping as u8);

        // "Bits 15-08: CLFLUSH line size (Value ∗ 8 = cache line size in bytes; used also by
        // CLFLUSHOPT)."
        self.clflush_size = ((ebx >> 8) & 0xff) * 8;
        self.cache_alignment = self.clflush_size;
        // Bits 31-24: Initial APIC ID.
        self.apicid = ebx >> 24;
        self.core_id = self.apicid;
    }

    fn fill_brand_info(&mut self) {
        let Some(CpuidResult {
            eax: eax1,
            ebx: ebx1,
            ecx: ecx1,
            edx: edx1,
        }) = cpuid(CpuidLeaf::ExtBrand1 as u32, 0)
        else {
            return;
        };
        let Some(CpuidResult {
            eax: eax2,
            ebx: ebx2,
            ecx: ecx2,
            edx: edx2,
        }) = cpuid(CpuidLeaf::ExtBrand2 as u32, 0)
        else {
            return;
        };
        let Some(CpuidResult {
            eax: eax3,
            ebx: ebx3,
            ecx: ecx3,
            edx: edx3,
        }) = cpuid(CpuidLeaf::ExtBrand3 as u32, 0)
        else {
            return;
        };

        #[rustfmt::skip]
        let brand = [
            eax1.to_le_bytes(), ebx1.to_le_bytes(), ecx1.to_le_bytes(), edx1.to_le_bytes(),
            eax2.to_le_bytes(), ebx2.to_le_bytes(), ecx2.to_le_bytes(), edx2.to_le_bytes(),
            eax3.to_le_bytes(), ebx3.to_le_bytes(), ecx3.to_le_bytes(), edx3.to_le_bytes(),
        ];
        let Ok(brand_cstr) = CStr::from_bytes_until_nul(brand.as_flattened()) else {
            return;
        };
        let Ok(brand_str) = brand_cstr.to_str() else {
            return;
        };

        self.model_name = brand_str.to_owned();
    }

    fn fill_cache_info(&mut self, vendor: CpuVendor) {
        match vendor {
            CpuVendor::Intel => self.fill_cache_info_intel(),
            CpuVendor::Amd | CpuVendor::Unknown => self.fill_cache_info_amd(),
        }
    }

    fn fill_cache_info_intel(&mut self) {
        let mut l1size = 0;
        let mut l2size = 0;
        let mut l3size = 0;

        for subleaf in 0.. {
            let Some(CpuidResult { eax, ebx, ecx, .. }) = cpuid(CpuidLeaf::Cache as u32, subleaf)
            else {
                break;
            };

            // Bits 04-00, Cache Type Field: Null - No more caches.
            if eax & 0x1f == 0 {
                break;
            }

            let system_coherency_line_size = ebx & 0xfff;
            let physical_line_partitions = (ebx >> 12) & 0x3ff;
            let ways_of_associativity = (ebx >> 22) & 0x3ff;
            let number_of_sets = ecx;

            let size = (system_coherency_line_size + 1)
                * (physical_line_partitions + 1)
                * (ways_of_associativity + 1)
                * (number_of_sets + 1);

            // Bits 07-05, Cache Level (starts at 1).
            match (eax >> 5) & 0x7 {
                1 => l1size += size,
                2 => l2size += size,
                3 => l3size += size,
                _ => (),
            }
        }

        if l3size != 0 {
            self.cache_size = Some(l3size / 1024);
        } else if l2size != 0 {
            self.cache_size = Some(l2size / 1024);
        } else if l1size != 0 {
            self.cache_size = Some(l1size / 1024);
        }
    }

    fn fill_cache_info_amd(&mut self) {
        let l1_cache_size =
            if let Some(CpuidResult { ecx, edx, .. }) = cpuid(CpuidLeaf::ExtCacheL1 as u32, 0) {
                // Bits 31-24, L1DcSize: L1 data cache size in KB.
                let l1d_cache_size = ecx >> 24;
                // Bits 31-24, L1IcSize: L1 instruction cache size KB.
                let l1i_cache_size = edx >> 24;
                l1d_cache_size + l1i_cache_size
            } else {
                0
            };

        let (l2_tlb_size, l2_cache_size) =
            if let Some(CpuidResult { ebx, ecx, .. }) = cpuid(CpuidLeaf::ExtCacheL2L3 as u32, 0) {
                // Bits 11-0, L2ITlb4KSize: L2 instruction TLB number of entries for 4-KB pages.
                let l2i_tlb_size = ebx & 0xfff;
                // Bits 27-16, L2DTlb4KSize: L2 data TLB number of entries for 4-KB pages.
                let l2d_tlb_size = (ebx >> 16) & 0xfff;
                // Bits 31-16, L2Size: L2 cache size in KB.
                let l2_cache_size = ecx >> 16;
                (l2i_tlb_size + l2d_tlb_size, l2_cache_size)
            } else {
                (0, 0)
            };

        // The Linux implementation here is quite odd, we just follow its logic:
        //  - If the L3 cache is present, it will be ignored on AMD CPUs [1] but it will be reported as
        //    the cache size on Intel CPUs [2].
        //  - The TLB size is never counted on Intel CPUs.
        //  - The L1 TLB is never counted on 64-bit AMD CPUs. But the L2 TLB will be counted.
        //
        // [1]: https://elixir.bootlin.com/linux/v6.16/source/arch/x86/kernel/cpu/common.c#L811
        // [2]: https://elixir.bootlin.com/linux/v6.16/source/arch/x86/kernel/cpu/cacheinfo.c#L359
        if l2_tlb_size != 0 {
            self.tlb_size = Some(l2_tlb_size);
        }
        if l2_cache_size != 0 {
            self.cache_size = Some(l2_cache_size);
        } else if l1_cache_size != 0 {
            self.cache_size = Some(l1_cache_size);
        }
    }

    fn fill_topology_info(&mut self) {
        let Some(CpuidResult { eax, edx, .. }) = cpuid(CpuidLeaf::Topology as u32, 0) else {
            return;
        };

        // "Bits 31-00: x2APIC ID of the current logical processor."
        self.apicid = edx;
        // "Bits 04-00: The number of bits that the x2APIC ID must be shifted to the right to
        // address instances of the next higher-scoped domain."
        self.core_id = self.apicid >> (eax & 0x1f);

        CPU_CORES.lock().insert(self.core_id);
    }

    fn fill_feature_info(&mut self) {
        macro_rules! parse_feature_bits {
            ($feature_word:expr; $($bit:literal => $name:literal,)*) => {{
                let feature_word = $feature_word;
                $(if feature_word & (1 << $bit) != 0 {
                    self.flags.push($name);
                })*
            }};
        }

        // The feature detection code below is based on
        // <https://github.com/torvalds/linux/blob/aaf724ed69264719550ec4f194d3ab17b886af9a/arch/x86/include/asm/cpufeatures.h>.
        // The specific commit is provided for the convenience of future synchronization with the
        // latest Linux implementation. Please don't forget to update the commit hash after you
        // modified the code below.
        //
        // Note that only Intel- and AMD-defined features are currently supported. We do not
        // include features defined by other vendors or defined by the Linux kernel.

        if let Some(CpuidResult { edx, .. }) = cpuid(0x00000001, 0) {
            parse_feature_bits!(edx;
                0 => "fpu",
                1 => "vme",
                2 => "de",
                3 => "pse",
                4 => "tsc",
                5 => "msr",
                6 => "pae",
                7 => "mce",
                8 => "cx8",
                9 => "apic",
                11 => "sep",
                12 => "mtrr",
                13 => "pge",
                14 => "mca",
                15 => "cmov",
                16 => "pat",
                17 => "pse36",
                18 => "pn",
                19 => "clflush",
                21 => "dts",
                22 => "acpi",
                23 => "mmx",
                24 => "fxsr",
                25 => "sse",
                26 => "sse2",
                27 => "ss",
                28 => "ht",
                29 => "tm",
                30 => "ia64",
                31 => "pbe",
            );
        }

        if let Some(CpuidResult { edx, .. }) = cpuid(0x80000001, 0) {
            parse_feature_bits!(edx;
                11 => "syscall",
                19 => "mp",
                20 => "nx",
                22 => "mmxext",
                25 => "fxsr_opt",
                26 => "pdpe1gb",
                27 => "rdtscp",
                29 => "lm",
                30 => "3dnowext",
                31 => "3dnow",
            );
        }

        if let Some(CpuidResult { ecx, .. }) = cpuid(0x00000001, 0) {
            parse_feature_bits!(ecx;
                0 => "pni",
                1 => "pclmulqdq",
                2 => "dtes64",
                3 => "monitor",
                4 => "ds_cpl",
                5 => "vmx",
                6 => "smx",
                7 => "est",
                8 => "tm2",
                9 => "ssse3",
                10 => "cid",
                11 => "sdbg",
                12 => "fma",
                13 => "cx16",
                14 => "xtpr",
                15 => "pdcm",
                17 => "pcid",
                18 => "dca",
                19 => "sse4_1",
                20 => "sse4_2",
                21 => "x2apic",
                22 => "movbe",
                23 => "popcnt",
                24 => "tsc_deadline_timer",
                25 => "aes",
                26 => "xsave",
                28 => "avx",
                29 => "f16c",
                30 => "rdrand",
                31 => "hypervisor",
            );
        }

        if let Some(CpuidResult { ecx, .. }) = cpuid(0x80000001, 0) {
            parse_feature_bits!(ecx;
                0 => "lahf_lm",
                1 => "cmp_legacy",
                2 => "svm",
                3 => "extapic",
                4 => "cr8_legacy",
                5 => "abm",
                6 => "sse4a",
                7 => "misalignsse",
                8 => "3dnowprefetch",
                9 => "osvw",
                10 => "ibs",
                11 => "xop",
                12 => "skinit",
                13 => "wdt",
                15 => "lwp",
                16 => "fma4",
                17 => "tce",
                19 => "nodeid_msr",
                21 => "tbm",
                22 => "topoext",
                23 => "perfctr_core",
                24 => "perfctr_nb",
                26 => "bpext",
                27 => "ptsc",
                28 => "perfctr_llc",
                29 => "mwaitx",
            );
        }

        if let Some(CpuidResult { ebx, .. }) = cpuid(0x00000007, 0) {
            parse_feature_bits!(ebx;
                0 => "fsgsbase",
                1 => "tsc_adjust",
                2 => "sgx",
                3 => "bmi1",
                4 => "hle",
                5 => "avx2",
                7 => "smep",
                8 => "bmi2",
                9 => "erms",
                10 => "invpcid",
                11 => "rtm",
                12 => "cqm",
                14 => "mpx",
                15 => "rdt_a",
                16 => "avx512f",
                17 => "avx512dq",
                18 => "rdseed",
                19 => "adx",
                20 => "smap",
                21 => "avx512ifma",
                23 => "clflushopt",
                24 => "clwb",
                25 => "intel_pt",
                26 => "avx512pf",
                27 => "avx512er",
                28 => "avx512cd",
                29 => "sha_ni",
                30 => "avx512bw",
                31 => "avx512vl",
            );
        }

        if let Some(CpuidResult { eax, .. }) = cpuid(0x0000000d, 1) {
            parse_feature_bits!(eax;
                0 => "xsaveopt",
                1 => "xsavec",
                2 => "xgetbv1",
                3 => "xsaves",
            );
        }

        if let Some(CpuidResult { eax, .. }) = cpuid(0x00000007, 1) {
            parse_feature_bits!(eax;
                4 => "avx_vnni",
                5 => "avx512_bf16",
                17 => "fred",
                18 => "kernel",
                26 => "lam",
            );
        }

        if let Some(CpuidResult { ebx, .. }) = cpuid(0x80000008, 0) {
            parse_feature_bits!(ebx;
                0 => "clzero",
                1 => "irperf",
                2 => "xsaveerptr",
                4 => "rdpru",
                9 => "wbnoinvd",
                23 => "amd_ppin",
                25 => "virt_ssbd",
                27 => "cppc",
                31 => "brs",
            );
        }

        if let Some(CpuidResult { eax, .. }) = cpuid(0x00000006, 0) {
            parse_feature_bits!(eax;
                0 => "dtherm",
                1 => "ida",
                2 => "arat",
                4 => "pln",
                6 => "pts",
                7 => "hwp",
                8 => "hwp_notify",
                9 => "hwp_act_window",
                10 => "hwp_epp",
                11 => "hwp_pkg_req",
                19 => "hfi",
            );
        }

        if let Some(CpuidResult { edx, .. }) = cpuid(0x8000000a, 0) {
            parse_feature_bits!(edx;
                0 => "npt",
                1 => "lbrv",
                2 => "svm_lock",
                3 => "nrip_save",
                4 => "tsc_scale",
                5 => "vmcb_clean",
                6 => "flushbyasid",
                7 => "decodeassists",
                10 => "pausefilter",
                12 => "pfthreshold",
                13 => "avic",
                15 => "v_vmsave_vmload",
                16 => "vgif",
                18 => "x2avic",
                20 => "v_spec_ctrl",
                25 => "vnmi",
            );
        }

        if let Some(CpuidResult { ecx, .. }) = cpuid(0x00000007, 0) {
            parse_feature_bits!(ecx;
                1 => "avx512vbmi",
                2 => "umip",
                3 => "pku",
                4 => "ospke",
                5 => "waitpkg",
                6 => "avx512_vbmi2",
                8 => "gfni",
                9 => "vaes",
                10 => "vpclmulqdq",
                11 => "avx512_vnni",
                12 => "avx512_bitalg",
                13 => "tme",
                14 => "avx512_vpopcntdq",
                16 => "la57",
                22 => "rdpid",
                24 => "bus_lock_detect",
                25 => "cldemote",
                27 => "movdiri",
                28 => "movdir64b",
                29 => "enqcmd",
                30 => "sgx_lc",
            );
        }

        if let Some(CpuidResult { ebx, .. }) = cpuid(0x80000007, 0) {
            parse_feature_bits!(ebx;
                0 => "overflow_recov",
                1 => "succor",
                3 => "smca",
            );
        }

        if let Some(CpuidResult { edx, .. }) = cpuid(0x00000007, 0) {
            parse_feature_bits!(edx;
                2 => "avx512_4vnniw",
                3 => "avx512_4fmaps",
                4 => "fsrm",
                8 => "avx512_vp2intersect",
                10 => "md_clear",
                14 => "serialize",
                16 => "tsxldtrk",
                18 => "pconfig",
                19 => "arch_lbr",
                20 => "ibt",
                22 => "amx_bf16",
                23 => "avx512_fp16",
                24 => "amx_tile",
                25 => "amx_int8",
                28 => "flush_l1d",
                29 => "arch_capabilities",
            );
        }

        if let Some(CpuidResult { eax, .. }) = cpuid(0x8000001f, 0) {
            parse_feature_bits!(eax;
                0 => "sme",
                1 => "sev",
                3 => "sev_es",
                4 => "sev_snp",
                14 => "debug_swap",
                28 => "svsm",
            );
        }
    }

    fn fill_address_info(&mut self) {
        let Some(CpuidResult { eax, .. }) = cpuid(CpuidLeaf::ExtAddrSizes as u32, 0) else {
            return;
        };

        // "Bits 07-00: #Physical Address Bits*."
        self.address_sizes.0 = eax & 0xff;
        // "Bits 15-08: #Linear Address Bits."
        self.address_sizes.1 = (eax >> 8) & 0xff;
    }
}

impl fmt::Display for CpuInformation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "processor\t: {}\n\
             vendor_id\t: {}\n\
             cpu family\t: {}\n\
             model\t\t: {}\n\
             model name\t: {}\n",
            self.processor, self.vendor_id, self.cpu_family, self.model, self.model_name,
        )?;

        if let Some(stepping) = self.stepping {
            writeln!(f, "stepping\t: {}", stepping)?;
        } else {
            writeln!(f, "stepping\t: unknown")?;
        }

        // TODO: Add the `microcode` field.

        if let Some(cpu_khz) = self.cpu_khz {
            writeln!(f, "cpu MHz\t\t: {}.{:03}", cpu_khz / 1000, cpu_khz % 1000)?;
        } else {
            writeln!(f, "cpu MHz\t\t: Unknown")?;
        }

        if let Some(cache_size) = self.cache_size {
            writeln!(f, "cache size\t: {} KB", cache_size)?;
        }

        // Note that we don't support NUMA now, so we assume that all CPUs are on the same package
        // (i.e., their physical IDs are all zeros).
        let siblings = num_cpus();
        let cpu_cores = CPU_CORES.lock().len();
        write!(
            f,
            "physical id\t: 0\n\
             siblings\t: {}\n\
             core id\t\t: {}\n\
             cpu cores\t: {}\n\
             apicid\t\t: {}\n\
             initial apicid\t: {}\n",
            siblings, self.core_id, cpu_cores, self.apicid, self.apicid
        )?;

        write!(
            f,
            "fpu\t\t: yes\n\
             fpu_exception\t: yes\n\
             cpuid level\t: {}\n\
             wp\t\t: yes\n",
            self.cpuid_level,
        )?;

        write!(f, "flags\t\t:")?;
        for flag in self.flags.iter() {
            write!(f, " {}", flag)?;
        }

        // TODO: Add the `vmx flags` field.

        // TODO: Fill the `bugs` field.
        write!(
            f,
            "\n\
             bugs\t\t:\n"
        )?;

        // TODO: Add the `bogomips` field.

        if let Some(tlb_size) = self.tlb_size {
            writeln!(f, "TLB size\t: {} 4K pages", tlb_size)?;
        }

        write!(
            f,
            "clflush size\t: {}\n\
             cache_alignment\t: {}\n\
             address sizes\t: {} bits physical, {} bits virtual\n",
            self.clflush_size, self.cache_alignment, self.address_sizes.0, self.address_sizes.1,
        )?;

        // TODO: Fill the `power management` field.
        writeln!(f, "power management:")?;

        Ok(())
    }
}
