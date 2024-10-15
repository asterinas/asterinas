// SPDX-License-Identifier: MPL-2.0

use alloc::{format, string::String, vec::Vec};

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
    physical_id: u32,
    siblings: u32,
    core_id: u32,
    cpu_cores: u32,
    apicid: u32,
    initial_apicid: u32,
    cpuid_level: u32,
    flags: String,
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
            cache_size: Self::get_cache_size(3).unwrap_or(0),
            physical_id: Self::get_physical_id().unwrap_or(0),
            siblings: Self::get_siblings_count().unwrap_or(0),
            core_id: Self::get_core_id(),
            cpu_cores: Self::get_cpu_cores(),
            apicid: Self::get_apicid(),
            initial_apicid: Self::get_initial_apicid(),
            cpuid_level: Self::get_cpuid_level(),
            flags: Self::get_cpu_flags(),
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
             physical id\t: {}\n\
             siblings\t: {}\n\
             core id\t\t: {}\n\
             cpu cores\t: {}\n\
             apicid\t\t: {}\n\
             initial apicid\t: {}\n\
             cpuid level\t: {}\n\
             flags\t\t: {}\n",
            self.processor,
            self.vendor_id,
            self.cpu_family,
            self.model,
            self.model_name,
            self.stepping,
            self.microcode,
            self.cpu_mhz,
            self.cache_size / 1024,
            self.physical_id,
            self.siblings,
            self.core_id,
            self.cpu_cores,
            self.apicid,
            self.initial_apicid,
            self.cpuid_level,
            self.flags
        )
    }

    fn get_vendor_id() -> String {
        let cpuid = cpuid!(0x0);
        let vendor = [
            cpuid.ebx.to_le_bytes(),
            cpuid.edx.to_le_bytes(),
            cpuid.ecx.to_le_bytes(),
        ]
        .concat();
        String::from_utf8(vendor).unwrap()
    }

    fn get_cpu_family() -> u32 {
        let cpuid = cpuid!(0x1);
        ((cpuid.eax >> 8) & 0xf) + (((cpuid.eax >> 20) & 0xff) << 4)
    }

    fn get_model() -> u32 {
        let cpuid = cpuid!(0x1);
        ((cpuid.eax >> 4) & 0xf) + (((cpuid.eax >> 16) & 0xf) << 4)
    }

    fn get_stepping() -> u32 {
        let cpuid = cpuid!(0x1);
        cpuid.eax & 0xf
    }

    fn get_model_name() -> String {
        let mut name = Vec::new();
        for i in 0x80000002..=0x80000004u32 {
            let cpuid = cpuid!(i);
            name.extend_from_slice(&cpuid.eax.to_le_bytes());
            name.extend_from_slice(&cpuid.ebx.to_le_bytes());
            name.extend_from_slice(&cpuid.ecx.to_le_bytes());
            name.extend_from_slice(&cpuid.edx.to_le_bytes());
        }
        String::from_utf8(name).unwrap()
    }

    fn get_microcode() -> u32 {
        let cpuid = cpuid!(0x1);
        cpuid.ecx
    }

    fn get_clock_speed() -> Option<u32> {
        let cpuid = cpuid!(0x16);
        if cpuid.eax == 0 {
            None
        } else {
            Some(cpuid.eax)
        }
    }

    /// Get cache size in KB
    fn get_cache_size(level: u32) -> Option<u32> {
        let cpuid = cpuid!(0x04, level);
        if cpuid.eax == 0 {
            None
        } else {
            let cache_size = ((cpuid.ebx >> 22) + 1)
                * (((cpuid.ebx >> 12) & 0x3ff) + 1)
                * ((cpuid.ebx & 0xfff) + 1)
                * cpuid.ecx;
            Some(cache_size)
        }
    }

    fn get_physical_id() -> Option<u32> {
        let cpuid = cpuid!(0x1);
        Some((cpuid.ebx >> 24) & 0xff)
    }

    fn get_siblings_count() -> Option<u32> {
        let cpuid = cpuid!(0x1);
        Some((cpuid.ebx >> 16) & 0xff)
    }

    fn get_core_id() -> u32 {
        let cpuid = cpuid!(0x1);
        (cpuid.ebx >> 24) & 0xff
    }

    fn get_cpu_cores() -> u32 {
        let cpuid = cpuid!(0x04);
        ((cpuid.eax >> 26) & 0x3f) + 1
    }

    fn get_apicid() -> u32 {
        let cpuid = cpuid!(0x1);
        (cpuid.ebx >> 24) & 0xff
    }

    fn get_initial_apicid() -> u32 {
        Self::get_apicid()
    }

    fn get_cpuid_level() -> u32 {
        let cpuid = cpuid!(0x0);
        cpuid.eax
    }

    fn get_cpu_flags() -> String {
        let cpuid = cpuid!(0x1);
        let mut flags = Vec::new();
        let edx_flags = [
            (0, "fpu"),
            (1, "vme"),
            (2, "de"),
            (3, "pse"),
            (4, "tsc"),
            (5, "msr"),
            (6, "pae"),
            (7, "mce"),
            (8, "cx8"),
            (9, "apic"),
            (11, "sep"),
            (12, "mtrr"),
            (13, "pge"),
            (14, "mca"),
            (15, "cmov"),
            (16, "pat"),
            (17, "pse-36"),
            (18, "psn"),
            (19, "clfsh"),
            (21, "ds"),
            (22, "acpi"),
            (23, "mmx"),
            (24, "fxsr"),
            (25, "sse"),
            (26, "sse2"),
            (27, "ss"),
            (28, "ht"),
            (29, "tm"),
            (30, "ia64"),
            (31, "pbe"),
        ];
        for (i, name) in edx_flags.iter() {
            if cpuid.edx & (1 << i) != 0 {
                flags.push(*name);
            }
        }
        flags.join(" ")
    }
}
