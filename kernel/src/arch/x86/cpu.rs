// SPDX-License-Identifier: MPL-2.0

use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

use ostd::{
    cpu::{cpuid, CpuException, CpuExceptionInfo, RawGeneralRegs, UserContext},
    Pod,
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

    fn set_tls_pointer(&mut self, tls: usize) {
        self.set_fsbase(tls);
    }

    fn tls_pointer(&self) -> usize {
        self.fsbase()
    }
}

/// General-purpose registers.
#[derive(Debug, Clone, Copy, Pod, Default)]
#[repr(C)]
pub struct GpRegs {
    pub rax: usize,
    pub rbx: usize,
    pub rcx: usize,
    pub rdx: usize,
    pub rsi: usize,
    pub rdi: usize,
    pub rbp: usize,
    pub rsp: usize,
    pub r8: usize,
    pub r9: usize,
    pub r10: usize,
    pub r11: usize,
    pub r12: usize,
    pub r13: usize,
    pub r14: usize,
    pub r15: usize,
    pub rip: usize,
    pub rflags: usize,
    pub fsbase: usize,
    pub gsbase: usize,
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
        $dst.fsbase = $src.fsbase;
        $dst.gsbase = $src.gsbase;
    };
}

impl GpRegs {
    pub fn copy_to_raw(&self, dst: &mut RawGeneralRegs) {
        copy_gp_regs!(self, dst);
    }

    pub fn copy_from_raw(&mut self, src: &RawGeneralRegs) {
        copy_gp_regs!(src, self);
    }
}

impl TryFrom<&CpuExceptionInfo> for PageFaultInfo {
    // [`Err`] indicates that the [`CpuExceptionInfo`] is not a page fault,
    // with no additional error information.
    type Error = ();

    fn try_from(value: &CpuExceptionInfo) -> Result<Self, ()> {
        if value.cpu_exception() != CpuException::PAGE_FAULT {
            return Err(());
        }

        const WRITE_ACCESS_MASK: usize = 0x1 << 1;
        const INSTRUCTION_FETCH_MASK: usize = 0x1 << 4;

        let required_perms = if value.error_code & INSTRUCTION_FETCH_MASK != 0 {
            VmPerms::EXEC
        } else if value.error_code & WRITE_ACCESS_MASK != 0 {
            VmPerms::WRITE
        } else {
            VmPerms::READ
        };

        Ok(PageFaultInfo {
            address: value.page_fault_addr,
            required_perms,
        })
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
        let tsc_info = cpuid.get_tsc_info()?;
        Some(
            (tsc_info.tsc_frequency().unwrap_or(0) / 1_000_000)
                .try_into()
                .unwrap(),
        )
    }

    /// Get cache size in KB
    fn get_cache_size() -> Option<u32> {
        let cpuid = cpuid::CpuId::new();
        let cache_info = cpuid.get_cache_info()?;

        for cache in cache_info {
            let desc = cache.desc();
            if let Some(size) = desc.split_whitespace().find(|word| {
                word.ends_with("KBytes") || word.ends_with("MBytes") || word.ends_with("GBytes")
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

    fn get_tlb_size() -> Option<u32> {
        let cpuid = cpuid::CpuId::new();
        let cache_info = cpuid.get_cache_info()?;

        for cache in cache_info {
            let desc = cache.desc();
            if let Some(size) = desc.split_whitespace().find(|word| word.ends_with("pages")) {
                let size_str = size.trim_end_matches("pages");
                let tlb_size = size_str.parse::<u32>().unwrap_or(0);
                return Some(tlb_size);
            }
        }

        None
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
        let cpuid = cpuid::CpuId::new();
        if let Some(basic_info) = cpuid.get_tsc_info() {
            basic_info.denominator()
        } else {
            0
        }
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
        cpuid.get_feature_info().unwrap().cflush_cache_line_size()
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
