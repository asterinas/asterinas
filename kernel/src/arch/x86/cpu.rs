// SPDX-License-Identifier: MPL-2.0

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

use ostd::{
    arch::tsc_freq,
    cpu::context::{cpuid, CpuException, PageFaultErrorCode, RawPageFaultInfo, UserContext},
    mm::Vaddr,
    Pod,
};

use crate::{
    arch::cpu::cpuid::VendorInfo, cpu::LinuxAbi, thread::exception::PageFaultInfo,
    vm::perms::VmPerms,
};

impl LinuxAbi for UserContext {
    fn syscall_num(&self) -> usize {
        self.rax()
    }

    fn syscall_ret(&self) -> usize {
        self.rax()
    }

    fn set_syscall_num(&mut self, num: usize) {
        self.set_rax(num);
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
            VmPerms::WRITE
        } else {
            VmPerms::READ
        };

        PageFaultInfo {
            address: raw_info.addr,
            required_perms,
        }
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

enum CpuVendor {
    Intel,
    Amd,
    Unknown,
}

impl From<&VendorInfo> for CpuVendor {
    fn from(info: &VendorInfo) -> Self {
        match info.as_str() {
            "GenuineIntel" => Self::Intel,
            "AuthenticAMD" => Self::Amd,
            _ => Self::Unknown,
        }
    }
}

/// CPU Information structure
///
/// Reference:
/// - https://www.felixcloutier.com/x86/cpuid
///
/// FIXME: The crate x86 works not well on AMD CPUs, so some information may be missing.
pub struct CpuInfo {
    processor: u32,
    vendor_id: String,
    cpu_family: u32,
    model: u32,
    model_name: String,
    stepping: u32,
    microcode: u32,
    cpu_mhz: u32,
    cache_size: u32,
    tlb_size: u32,
    physical_id: u32,
    siblings: u32,
    core_id: u32,
    cpu_cores: u32,
    apicid: u32,
    initial_apicid: u32,
    cpuid_level: u32,
    flags: String,
    bugs: String,
    clflush_size: u8,
    cache_alignment: u32,
    address_sizes: String,
    power_management: String,
}

impl CpuInfo {
    pub fn new(processor_id: u32) -> Self {
        Self {
            processor: processor_id,
            vendor_id: Self::get_vendor_id(),
            cpu_family: Self::get_cpu_family(),
            model: Self::get_model(),
            model_name: Self::get_model_name(),
            stepping: Self::get_stepping(),
            microcode: Self::get_microcode(),
            cpu_mhz: Self::get_clock_speed().unwrap_or(0),
            cache_size: Self::get_cache_size().unwrap_or(0),
            tlb_size: Self::get_tlb_size().unwrap_or(0),
            physical_id: Self::get_physical_id().unwrap_or(0),
            siblings: Self::get_siblings_count().unwrap_or(0),
            core_id: Self::get_core_id(),
            cpu_cores: Self::get_cpu_cores(),
            apicid: Self::get_apicid(),
            initial_apicid: Self::get_initial_apicid(),
            cpuid_level: Self::get_cpuid_level(),
            flags: Self::get_cpu_flags(),
            bugs: Self::get_cpu_bugs(),
            // bogomips: Self::get_bogomips(),
            clflush_size: Self::get_clflush_size(),
            cache_alignment: Self::get_cache_alignment(),
            address_sizes: Self::get_address_sizes(),
            power_management: Self::get_power_management(),
        }
    }

    /// Collect and format CPU information into a `String`
    pub fn collect_cpu_info(&self) -> String {
        format!(
            "processor\t: {}\n\
             vendor_id\t: {}\n\
             cpu family\t: {}\n\
             model\t\t: {}\n\
             model name\t: {}\n\
             stepping\t: {}\n\
             microcode\t: 0x{:x}\n\
             cpu MHz\t\t: {}\n\
             cache size\t: {} KB\n\
             TLB size\t: {} 4K pages\n\
             physical id\t: {}\n\
             siblings\t: {}\n\
             core id\t\t: {}\n\
             cpu cores\t: {}\n\
             apicid\t\t: {}\n\
             initial apicid\t: {}\n\
             cpuid level\t: {}\n\
             flags\t\t: {}\n\
             bugs\t\t: {}\n\
             clflush size\t: {} bytes\n\
             cache_alignment\t: {} bytes\n\
             address sizes\t: {}\n\
             power management: {}\n",
            self.processor,
            self.vendor_id,
            self.cpu_family,
            self.model,
            self.model_name,
            self.stepping,
            self.microcode,
            self.cpu_mhz,
            self.cache_size / 1024,
            self.tlb_size,
            self.physical_id,
            self.siblings,
            self.core_id,
            self.cpu_cores,
            self.apicid,
            self.initial_apicid,
            self.cpuid_level,
            self.flags,
            self.bugs,
            self.clflush_size,
            self.cache_alignment,
            self.address_sizes,
            self.power_management
        )
    }

    fn get_vendor_id() -> String {
        let cpuid = cpuid::CpuId::new();
        cpuid.get_vendor_info().unwrap().to_string()
    }

    fn get_cpu_family() -> u32 {
        let cpuid = cpuid::CpuId::new();
        let feature_info = cpuid.get_feature_info().unwrap();
        feature_info.family_id().into()
    }

    fn get_model() -> u32 {
        let cpuid = cpuid::CpuId::new();
        let feature_info = cpuid.get_feature_info().unwrap();
        feature_info.model_id().into()
    }

    fn get_stepping() -> u32 {
        let cpuid = cpuid::CpuId::new();
        let feature_info = cpuid.get_feature_info().unwrap();
        feature_info.stepping_id().into()
    }

    fn get_model_name() -> String {
        let cpuid = cpuid::CpuId::new();
        let brand_string = cpuid.get_processor_brand_string().unwrap();
        brand_string.as_str().to_string()
    }

    fn get_microcode() -> u32 {
        let cpuid = cpuid::cpuid!(0x1);
        cpuid.ecx
    }

    fn get_clock_speed() -> Option<u32> {
        let cpuid = cpuid::CpuId::new();
        let vendor_info = cpuid.get_vendor_info()?;
        match CpuVendor::from(&vendor_info) {
            CpuVendor::Intel => {
                let tsc_info = cpuid.get_tsc_info()?;
                Some(
                    (tsc_info.tsc_frequency().unwrap_or(0) / 1_000_000)
                        .try_into()
                        .unwrap(),
                )
            }
            CpuVendor::Amd | CpuVendor::Unknown => {
                let tsc_freq_hz = tsc_freq(); // always > 0
                Some((tsc_freq_hz / 1_000_000) as u32)
            }
        }
    }

    /// Get cache size in KB
    fn get_cache_size() -> Option<u32> {
        let cpuid = cpuid::CpuId::new();
        let vendor_info = cpuid.get_vendor_info()?;
        match CpuVendor::from(&vendor_info) {
            CpuVendor::Intel => {
                let cache_info = cpuid.get_cache_info()?;
                for cache in cache_info {
                    let desc = cache.desc();
                    if let Some(size) = desc.split_whitespace().find(|word| {
                        word.ends_with("KBytes")
                            || word.ends_with("MBytes")
                            || word.ends_with("GBytes")
                    }) {
                        let size_str = size
                            .trim_end_matches(&['K', 'M', 'G'][..])
                            .trim_end_matches("Bytes");
                        let cache_size = size_str.parse::<u32>().unwrap_or(0);
                        let cache_size = match size.chars().last().unwrap() {
                            'K' => cache_size * 1024,
                            'M' => cache_size * 1024 * 1024,
                            'G' => cache_size * 1024 * 1024 * 1024,
                            _ => cache_size,
                        };
                        return Some(cache_size);
                    }
                }
                None
            }
            CpuVendor::Amd => {
                let cache = cpuid.get_l2_l3_cache_and_tlb_info()?;
                Some(cache.l2cache_size() as u32 * 1024)
            }
            CpuVendor::Unknown => None,
        }
    }

    fn get_tlb_size() -> Option<u32> {
        let cpuid = cpuid::CpuId::new();
        let vendor_info = cpuid.get_vendor_info()?;
        match CpuVendor::from(&vendor_info) {
            CpuVendor::Intel => {
                let cache_info = cpuid.get_cache_info()?;
                for cache in cache_info {
                    let desc = cache.desc();
                    if let Some(size) = desc.split_whitespace().find(|word| word.ends_with("pages"))
                    {
                        let size_str = size.trim_end_matches("pages");
                        let tlb_size = size_str.parse::<u32>().unwrap_or(0);
                        return Some(tlb_size);
                    }
                }
                None
            }
            CpuVendor::Amd => {
                let cache = cpuid.get_l2_l3_cache_and_tlb_info()?;
                Some(cache.dtlb_4k_size() as u32)
            }
            CpuVendor::Unknown => None,
        }
    }

    fn get_physical_id() -> Option<u32> {
        let cpuid = cpuid::CpuId::new();
        let feature_info = cpuid.get_feature_info()?;
        Some(feature_info.initial_local_apic_id().into())
    }

    fn get_siblings_count() -> Option<u32> {
        let cpuid = cpuid::CpuId::new();
        let feature_info = cpuid.get_feature_info()?;
        Some(feature_info.max_logical_processor_ids().into())
    }

    fn get_core_id() -> u32 {
        let cpuid = cpuid::CpuId::new();
        let feature_info = cpuid.get_feature_info().unwrap();
        feature_info.initial_local_apic_id().into()
    }

    fn get_cpu_cores() -> u32 {
        let cpuid = cpuid::CpuId::new();
        let feature_info = cpuid.get_feature_info().unwrap();
        feature_info.max_logical_processor_ids().into()
    }

    fn get_apicid() -> u32 {
        let cpuid = cpuid::CpuId::new();
        let feature_info = cpuid.get_feature_info().unwrap();
        feature_info.initial_local_apic_id().into()
    }

    fn get_initial_apicid() -> u32 {
        Self::get_apicid()
    }

    fn get_cpuid_level() -> u32 {
        cpuid::cpuid!(0x0).eax
    }

    fn get_cpu_flags() -> String {
        let cpuid = cpuid::CpuId::new();
        let feature_info = cpuid.get_feature_info().unwrap();
        let mut flags = Vec::new();
        if feature_info.has_fpu() {
            flags.push("fpu");
        }
        if feature_info.has_vme() {
            flags.push("vme");
        }
        if feature_info.has_de() {
            flags.push("de");
        }
        if feature_info.has_pse() {
            flags.push("pse");
        }
        if feature_info.has_tsc() {
            flags.push("tsc");
        }
        if feature_info.has_msr() {
            flags.push("msr");
        }
        if feature_info.has_pae() {
            flags.push("pae");
        }
        if feature_info.has_mce() {
            flags.push("mce");
        }
        if feature_info.has_cmpxchg8b() {
            flags.push("cx8");
        }
        if feature_info.has_apic() {
            flags.push("apic");
        }
        if feature_info.has_de() {
            flags.push("sep");
        }
        if feature_info.has_mtrr() {
            flags.push("mtrr");
        }
        if feature_info.has_pge() {
            flags.push("pge");
        }
        if feature_info.has_mca() {
            flags.push("mca");
        }
        if feature_info.has_cmov() {
            flags.push("cmov");
        }
        if feature_info.has_pat() {
            flags.push("pat");
        }
        if feature_info.has_pse36() {
            flags.push("pse-36");
        }
        if feature_info.has_psn() {
            flags.push("psn");
        }
        if feature_info.has_clflush() {
            flags.push("clfsh");
        }
        if feature_info.has_ds() {
            flags.push("ds");
        }
        if feature_info.has_acpi() {
            flags.push("acpi");
        }
        if feature_info.has_mmx() {
            flags.push("mmx");
        }
        if feature_info.has_ds() {
            flags.push("fxsr");
        }
        if feature_info.has_sse() {
            flags.push("sse");
        }
        if feature_info.has_sse2() {
            flags.push("sse2");
        }
        if feature_info.has_ss() {
            flags.push("ss");
        }
        if feature_info.has_htt() {
            flags.push("ht");
        }
        if feature_info.has_tm() {
            flags.push("tm");
        }
        if feature_info.has_pbe() {
            flags.push("pbe");
        }
        flags.join(" ")
    }

    // FIXME: https://github.com/torvalds/linux/blob/master/tools/arch/x86/include/asm/cpufeatures.h#L505
    fn get_cpu_bugs() -> String {
        " ".to_string()
    }

    fn get_clflush_size() -> u8 {
        let cpuid = cpuid::CpuId::new();
        cpuid.get_feature_info().unwrap().cflush_cache_line_size() * 8
    }

    fn get_cache_alignment() -> u32 {
        let cpuid = cpuid::CpuId::new();
        if let Some(cache_info) = cpuid.get_cache_info() {
            for cache in cache_info {
                let desc = cache.desc();
                if let Some(alignment) = desc
                    .split_whitespace()
                    .find(|word| word.ends_with("byte line size"))
                {
                    let alignment_str = alignment.trim_end_matches(" byte line size");
                    if let Ok(alignment) = alignment_str.parse::<u32>() {
                        return alignment;
                    }
                }
            }
        }

        64
    }

    fn get_address_sizes() -> String {
        let leaf = cpuid::cpuid!(0x80000008); // Extended Function CPUID Information
        let physical_address_bits = (leaf.eax & 0xFF) as u32;
        let virtual_address_bits = ((leaf.eax >> 8) & 0xFF) as u32;
        format!(
            "{} bits physical, {} bits virtual",
            physical_address_bits, virtual_address_bits
        )
    }

    // FIXME: add power management information
    fn get_power_management() -> String {
        " ".to_string()
    }
}
