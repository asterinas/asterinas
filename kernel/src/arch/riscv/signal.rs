// SPDX-License-Identifier: MPL-2.0

use ostd::arch::cpu::context::{CpuException, UserContext};

use crate::process::signal::{SignalContext, sig_num::SigNum, signals::fault::FaultSignal};

impl SignalContext for UserContext {
    fn set_arguments(&mut self, sig_num: SigNum, siginfo_addr: usize, ucontext_addr: usize) {
        self.set_a0(sig_num.as_u8() as usize);
        self.set_a1(siginfo_addr);
        self.set_a2(ucontext_addr);
    }
}

impl From<&CpuException> for FaultSignal {
    fn from(exception: &CpuException) -> Self {
        use CpuException::*;

        use crate::process::signal::constants::*;

        // FIXME: All the `None` addresses here should be the value of `sepc`
        // CSR on trap. So we should either encode that in `CpuException` or
        // pass it as an additional parameter here.
        let (num, code, addr) = match exception {
            InstructionMisaligned => (SIGBUS, BUS_ADRALN, None),
            InstructionFault => (SIGSEGV, SEGV_ACCERR, None),
            IllegalInstruction(_) => (SIGILL, ILL_ILLOPC, None),
            Breakpoint => (SIGTRAP, TRAP_BRKPT, None),
            LoadMisaligned(_) | StoreMisaligned(_) => (SIGBUS, BUS_ADRALN, None),
            LoadFault(_) | StoreFault(_) => (SIGSEGV, SEGV_ACCERR, None),
            UserEnvCall => unreachable!(),
            SupervisorEnvCall => (SIGILL, ILL_ILLTRP, None),
            InstructionPageFault(addr) | LoadPageFault(addr) | StorePageFault(addr) => {
                (SIGSEGV, SEGV_MAPERR, Some(*addr as u64))
            }
            Unknown => (SIGILL, ILL_ILLTRP, None),
        };

        FaultSignal::new(num, code, addr)
    }
}
