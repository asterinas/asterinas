// SPDX-License-Identifier: MPL-2.0

use ostd::{
    cpu::{CpuExceptionInfo, RawGeneralRegs, UserContext},
    Pod,
};

use crate::{cpu::LinuxAbi, thread::exception::PageFaultInfo, vm::perms::VmPerms};

impl LinuxAbi for UserContext {
    fn syscall_num(&self) -> usize {
        self.a7()
    }

    fn syscall_ret(&self) -> usize {
        self.a0()
    }

    fn set_syscall_ret(&mut self, ret: usize) {
        self.set_a0(ret)
    }

    fn syscall_args(&self) -> [usize; 6] {
        [
            self.a0(),
            self.a1(),
            self.a2(),
            self.a3(),
            self.a4(),
            self.a5(),
        ]
    }

    fn set_tls_pointer(&mut self, tls: usize) {
        self.set_tp(tls);
    }

    fn tls_pointer(&self) -> usize {
        self.tp()
    }
}

/// General-purpose registers.
#[derive(Debug, Clone, Copy, Pod, Default)]
#[repr(C)]
pub struct GpRegs {
    pub zero: usize,
    pub ra: usize,
    pub sp: usize,
    pub gp: usize,
    pub tp: usize,
    pub t0: usize,
    pub t1: usize,
    pub t2: usize,
    pub s0: usize,
    pub s1: usize,
    pub a0: usize,
    pub a1: usize,
    pub a2: usize,
    pub a3: usize,
    pub a4: usize,
    pub a5: usize,
    pub a6: usize,
    pub a7: usize,
    pub s2: usize,
    pub s3: usize,
    pub s4: usize,
    pub s5: usize,
    pub s6: usize,
    pub s7: usize,
    pub s8: usize,
    pub s9: usize,
    pub s10: usize,
    pub s11: usize,
    pub t3: usize,
    pub t4: usize,
    pub t5: usize,
    pub t6: usize,
}

macro_rules! copy_gp_regs {
    ($src: ident, $dst: ident) => {
        $dst.zero = $src.zero;
        $dst.ra = $src.ra;
        $dst.sp = $src.sp;
        $dst.gp = $src.gp;
        $dst.tp = $src.tp;
        $dst.t0 = $src.t0;
        $dst.t1 = $src.t1;
        $dst.t2 = $src.t2;
        $dst.s0 = $src.s0;
        $dst.s1 = $src.s1;
        $dst.a0 = $src.a0;
        $dst.a1 = $src.a1;
        $dst.a2 = $src.a2;
        $dst.a3 = $src.a3;
        $dst.a4 = $src.a4;
        $dst.a5 = $src.a5;
        $dst.a6 = $src.a6;
        $dst.a7 = $src.a7;
        $dst.s2 = $src.s2;
        $dst.s3 = $src.s3;
        $dst.s4 = $src.s4;
        $dst.s5 = $src.s5;
        $dst.s6 = $src.s6;
        $dst.s7 = $src.s7;
        $dst.s8 = $src.s8;
        $dst.s9 = $src.s9;
        $dst.s10 = $src.s10;
        $dst.s11 = $src.s11;
        $dst.t3 = $src.t3;
        $dst.t4 = $src.t4;
        $dst.t5 = $src.t5;
        $dst.t6 = $src.t6;
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
        use riscv::register::scause::Exception;

        let required_perms = match value.cpu_exception() {
            Exception::InstructionPageFault => VmPerms::EXEC,
            Exception::LoadPageFault => VmPerms::READ,
            Exception::StorePageFault => VmPerms::WRITE,
            _ => return Err(()),
        };

        Ok(PageFaultInfo {
            address: value.page_fault_addr,
            required_perms,
        })
    }
}
