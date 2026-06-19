use crate::arch::{
    cpu::context::FpuContext,
    vm::{
        vmx::Msr,
        x86::{read_cr2_raw, write_cr2_raw},
    },
};

#[derive(Debug, Clone)]
/// Host Contexts that can't be auto loaded/saved by VMCS
pub struct HostContext {
    msrs: HostRunMsrs,
    fpu: FpuContext,
    cr2: u64,
}

impl HostContext {
    pub fn save() -> Self {
        let mut fpu = FpuContext::new();
        fpu.save();
        Self {
            msrs: HostRunMsrs::read_current(),
            fpu,
            cr2: read_cr2_raw(),
        }
    }

    pub fn load(mut self) {
        write_cr2_raw(self.cr2);
        self.fpu.load();
        self.msrs.restore();
    }
}

#[derive(Debug, Clone, Copy)]
struct HostRunMsrs {
    star: u64,
    lstar: u64,
    cstar: u64,
    syscall_mask: u64,
    kernel_gs_base: u64,
}

impl HostRunMsrs {
    fn read_current() -> Self {
        Self {
            star: Msr::IA32_STAR.read(),
            lstar: Msr::IA32_LSTAR.read(),
            cstar: Msr::IA32_CSTAR.read(),
            syscall_mask: Msr::IA32_FMASK.read(),
            kernel_gs_base: Msr::IA32_KERNEL_GSBASE.read(),
        }
    }

    fn restore(self) {
        Msr::IA32_STAR.write(self.star);
        Msr::IA32_LSTAR.write(self.lstar);
        Msr::IA32_CSTAR.write(self.cstar);
        Msr::IA32_FMASK.write(self.syscall_mask);
        Msr::IA32_KERNEL_GSBASE.write(self.kernel_gs_base);
    }
}
