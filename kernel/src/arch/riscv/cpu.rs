// SPDX-License-Identifier: MPL-2.0

use core::fmt;

use ostd::{
    Pod,
    arch::cpu::context::{CpuException, UserContext},
    cpu::PinCurrentCpu,
    task::DisabledPreemptGuard,
    user::UserContextApi,
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
}

macro_rules! copy_gp_regs {
    ($src: ident, $dst: ident) => {
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

/// Represents the context of a signal handler.
///
/// This contains the context saved before a signal handler is invoked; it will be restored by
/// `sys_rt_sigreturn`.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.15.7/source/arch/riscv/include/uapi/asm/sigcontext.h#L30>
#[repr(C)]
#[repr(align(16))]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct SigContext {
    pc: usize,
    ra: usize,
    sp: usize,
    gp: usize,
    tp: usize,
    t0: usize,
    t1: usize,
    t2: usize,
    s0: usize,
    s1: usize,
    a0: usize,
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
    a5: usize,
    a6: usize,
    a7: usize,
    s2: usize,
    s3: usize,
    s4: usize,
    s5: usize,
    s6: usize,
    s7: usize,
    s8: usize,
    s9: usize,
    s10: usize,
    s11: usize,
    t3: usize,
    t4: usize,
    t5: usize,
    t6: usize,
    // In RISC-V, the signal stack layout places the FPU context directly
    // after the general-purpose registers.
}

impl SigContext {
    pub fn copy_user_regs_to(&self, dst: &mut UserContext) {
        let gp_regs = dst.general_regs_mut();
        copy_gp_regs!(self, gp_regs);
        dst.set_instruction_pointer(self.pc);
    }

    pub fn copy_user_regs_from(&mut self, src: &UserContext) {
        let gp_regs = src.general_regs();
        copy_gp_regs!(gp_regs, self);
        self.pc = src.instruction_pointer();
    }
}

impl TryFrom<&CpuException> for PageFaultInfo {
    // [`Err`] indicates that the [`CpuException`] is not a page fault, with no
    // additional error information.
    type Error = ();

    fn try_from(value: &CpuException) -> Result<Self, ()> {
        use CpuException::*;

        let (fault_addr, required_perms) = match value {
            InstructionPageFault(addr) => (addr, VmPerms::EXEC),
            LoadPageFault(addr) => (addr, VmPerms::READ),
            // On riscv64, writable pages must also be readable.
            // Reference: <https://riscv.github.io/riscv-isa-manual/snapshot/privileged/#translation>.
            StorePageFault(addr) => (addr, VmPerms::READ | VmPerms::WRITE),
            _ => return Err(()),
        };

        Ok(PageFaultInfo::new(*fault_addr, required_perms))
    }
}

/// CPU information to be shown in `/proc/cpuinfo`.
///
/// Different CPUs may have different information, such as the core ID. Therefore, [`Self::new`]
/// should be called on every CPU.
//
// TODO: Implement CPU information retrieval on RISC-V platforms.
pub struct CpuInformation {
    processor: u32,
}

impl CpuInformation {
    /// Constructs the information for the current CPU.
    pub fn new(guard: &DisabledPreemptGuard) -> Self {
        Self {
            processor: guard.current_cpu().into(),
        }
    }
}

impl fmt::Display for CpuInformation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "processor\t: {}", self.processor)
    }
}
