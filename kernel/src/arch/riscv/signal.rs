// SPDX-License-Identifier: MPL-2.0

use ostd::arch::cpu::context::{CpuException, UserContext};

use crate::process::signal::{
    constants::*, sig_num::SigNum, signals::fault::FaultSignal, SignalContext,
};

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
        let (num, code, addr) = match exception {
            InstructionMisaligned => (SIGBUS, BUS_ADRALN, None),
            LoadMisaligned(addr) | StoreMisaligned(addr) => {
                (SIGBUS, BUS_ADRALN, Some(*addr as u64))
            }
            InstructionFault => (SIGSEGV, SEGV_ACCERR, None),
            LoadFault(addr) | StoreFault(addr) => (SIGSEGV, SEGV_ACCERR, Some(*addr as u64)),
            InstructionPageFault(addr) | LoadPageFault(addr) | StorePageFault(addr) => {
                (SIGSEGV, SEGV_MAPERR, Some(*addr as u64))
            }
            IllegalInstruction(_) => (SIGILL, ILL_ILLOPC, None),
            Breakpoint => (SIGTRAP, TRAP_BRKPT, None),
            e => panic!("{e:?} cannot be handled via signals ({exception:?})"),
        };

        FaultSignal::new(num, code, addr)
    }
}
