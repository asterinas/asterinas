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
        // The AArch64 signal-handler ABI passes arguments in `x0`, `x1`, `x2`.
        self.set_x(0, sig_num.as_u8() as usize);
        self.set_x(1, siginfo_addr);
        self.set_x(2, ucontext_addr);
    }
}

impl ToFaultSignal for CpuException {
    fn to_fault_signal(&self, user_ctx: &UserContext) -> Option<FaultSignal> {
        use CpuException::*;

        use crate::process::signal::constants::*;

        let pc = user_ctx.instruction_pointer() as u64;

        let (num, code, addr) = match self {
            InstructionAbort(info) | DataAbort(info) => {
                if info.is_page_fault() {
                    // FIXME: Use `SEGV_ACCERR` for permission faults within an
                    // existing mapping.
                    (SIGSEGV, SEGV_MAPERR, info.far as u64)
                } else {
                    (SIGBUS, BUS_ADRERR, info.far as u64)
                }
            }
            PcAlignment | SpAlignment => (SIGBUS, BUS_ADRALN, pc),
            IllegalState => (SIGILL, ILL_ILLOPC, pc),
            Breakpoint => (SIGTRAP, TRAP_BRKPT, pc),
            Unknown(_) => (SIGILL, ILL_ILLTRP, pc),
            // A system call is not a fault.
            Syscall => return None,
        };

        Some(FaultSignal::new(num, code, Some(addr)))
    }
}
