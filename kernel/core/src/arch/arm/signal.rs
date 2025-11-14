// SPDX-License-Identifier: MPL-2.0

use ostd::{
    arch::cpu::context::{CpuException, UserContext},
    user::UserContextApi,
};

use crate::{
    process::signal::{SignalContext, sig_num::SigNum, signals::fault::FaultSignal},
    thread::exception::ToFaultSignal,
};

impl SignalContext for UserContext {
    fn set_arguments(&mut self, sig_num: SigNum, siginfo_addr: usize, ucontext_addr: usize) {
        self.set_x0(sig_num.as_u8() as usize);
        self.set_x1(siginfo_addr);
        self.set_x2(ucontext_addr);
    }
}

impl ToFaultSignal for CpuException {
    fn to_fault_signal(&self, user_ctx: &UserContext) -> Option<FaultSignal> {
        use crate::process::signal::constants::*;

        let elr = user_ctx.instruction_pointer() as u64;

        let (num, code, addr) = match self {
            CpuException::Unknown
            | CpuException::WfiInstruction
            | CpuException::FpuInstruction
            | CpuException::IllegalState
            | CpuException::SystemInstruction => (SIGILL, ILL_ILLOPC, Some(elr)),
            CpuException::InstructionAbort { address }
            | CpuException::DataAbort { address, .. } => {
                // FIXME: Derive the signal number and code from the error code.
                // See <https://elixir.bootlin.com/linux/v7.0/source/arch/arm64/mm/fault.c#L861>.
                (SIGSEGV, SEGV_MAPERR, Some(*address as u64))
            }
            CpuException::PcAlignmentFault => (SIGBUS, BUS_ADRALN, Some(elr)),
            CpuException::SpAlignmentFault => {
                (SIGBUS, BUS_ADRALN, Some(user_ctx.stack_pointer() as u64))
            }
            CpuException::FpuException => {
                // TODO: Derive the code from the floating-point status.
                (SIGFPE, FPE_FLTDIV, Some(elr))
            }
            CpuException::SoftwareStep => (SIGTRAP, TRAP_TRACE, Some(elr)),
            CpuException::BrkInstruction => (SIGTRAP, TRAP_BRKPT, Some(elr)),

            CpuException::Breakpoint | CpuException::Watchpoint => {
                // TODO: Properly handle these exceptions once hardware debugging is supported.
                return None;
            }
            CpuException::SvcInstruction | CpuException::SErrorInterrupt => return None,
        };

        Some(FaultSignal::new(num, code, addr))
    }
}
