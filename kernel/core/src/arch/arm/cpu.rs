// SPDX-License-Identifier: MPL-2.0

use core::fmt;

use ostd::{
    arch::cpu::context::UserContext, cpu::PinCurrentCpu, task::DisabledPreemptGuard,
    user::UserContextApi,
};

use crate::cpu::LinuxAbi;

impl LinuxAbi for UserContext {
    fn syscall_num(&self) -> usize {
        self.x8()
    }

    fn syscall_ret(&self) -> usize {
        self.x0()
    }

    fn set_syscall_ret(&mut self, ret: usize) {
        self.set_x0(ret)
    }

    fn syscall_args(&self) -> [usize; 6] {
        [
            self.x0(),
            self.x1(),
            self.x2(),
            self.x3(),
            self.x4(),
            self.x5(),
        ]
    }
}

macro_rules! copy_gp_regs {
    ($regs: ident) => {
        copy_reg!(0, x0, $regs);
        copy_reg!(1, x1, $regs);
        copy_reg!(2, x2, $regs);
        copy_reg!(3, x3, $regs);
        copy_reg!(4, x4, $regs);
        copy_reg!(5, x5, $regs);
        copy_reg!(6, x6, $regs);
        copy_reg!(7, x7, $regs);
        copy_reg!(8, x8, $regs);
        copy_reg!(9, x9, $regs);
        copy_reg!(10, x10, $regs);
        copy_reg!(11, x11, $regs);
        copy_reg!(12, x12, $regs);
        copy_reg!(13, x13, $regs);
        copy_reg!(14, x14, $regs);
        copy_reg!(15, x15, $regs);
        copy_reg!(16, x16, $regs);
        copy_reg!(17, x17, $regs);
        copy_reg!(18, x18, $regs);
        copy_reg!(19, x19, $regs);
        copy_reg!(20, x20, $regs);
        copy_reg!(21, x21, $regs);
        copy_reg!(22, x22, $regs);
        copy_reg!(23, x23, $regs);
        copy_reg!(24, x24, $regs);
        copy_reg!(25, x25, $regs);
        copy_reg!(26, x26, $regs);
        copy_reg!(27, x27, $regs);
        copy_reg!(28, x28, $regs);
        copy_reg!(29, x29, $regs);
        copy_reg!(30, x30, $regs);
    };
}

/// Represents the context of a signal handler.
///
/// This contains the context saved before a signal handler is invoked; it will be restored by
/// `sys_rt_sigreturn`.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.15.7/source/arch/arm64/include/uapi/asm/sigcontext.h#L28>
#[repr(C)]
#[repr(align(16))]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(crate) struct SigContext {
    fault_address: u64,
    regs: [u64; 31],
    sp: u64,
    pc: u64,
    pstate: u64,
    _pad: u64,
    // In ARM, the signal stack layout places the FPU context directly
    // after the general-purpose registers.
}

impl SigContext {
    pub(crate) fn copy_user_regs_to(&self, dst: &mut UserContext) {
        macro_rules! copy_reg {
            ($idx: literal, $name: ident, $regs: ident) => {
                $regs.$name = self.regs[$idx] as usize;
            };
        }

        let gp_regs = dst.general_regs_mut();
        copy_gp_regs!(gp_regs);
        dst.set_stack_pointer(self.sp as usize);
        dst.set_instruction_pointer(self.pc as usize);
        dst.set_process_state(self.pstate as usize);
    }

    pub(crate) fn copy_user_regs_from(&mut self, src: &UserContext) {
        macro_rules! copy_reg {
            ($idx:literal, $name:ident, $regs: ident) => {
                self.regs[$idx] = $regs.$name as u64;
            };
        }

        let gp_regs = src.general_regs();
        copy_gp_regs!(gp_regs);
        self.sp = src.stack_pointer() as u64;
        self.pc = src.instruction_pointer() as u64;
        self.pstate = src.process_state() as u64;
    }
}

/// CPU information to be shown in `/proc/cpuinfo`.
///
/// Different CPUs may have different information, such as the core ID. Therefore, [`Self::new`]
/// should be called on every CPU.
//
// TODO: Implement CPU information retrieval on ARM platforms.
pub(crate) struct CpuInformation {
    processor: u32,
}

impl CpuInformation {
    /// Constructs the information for the current CPU.
    pub(crate) fn new(guard: &DisabledPreemptGuard) -> Self {
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
