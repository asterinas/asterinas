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
        // ARM64 Linux signal handler ABI: x0=sig_num, x1=siginfo, x2=ucontext
        self.set_x0(sig_num.as_u8() as usize);
        self.set_x1(siginfo_addr);
        self.set_x2(ucontext_addr);
    }
}

impl ToFaultSignal for CpuException {
    fn to_fault_signal(&self, user_ctx: &UserContext) -> Option<FaultSignal> {
        use CpuException::*;

        use crate::process::signal::constants::*;

        let elr = user_ctx.instruction_pointer() as u64;

        let (num, code, addr) = match self {
            InstructionPageFault(addr) | LoadPageFault(addr) | StorePageFault(addr) => {
                // FIXME: The code should depend on whether the faulting address is covered by a
                // mapping, not just on the exception type.
                (SIGSEGV, SEGV_MAPERR, *addr as u64)
            }
            Unknown => (SIGILL, ILL_ILLOPC, elr),
        };

        Some(FaultSignal::new(num, code, Some(addr)))
    }
}
