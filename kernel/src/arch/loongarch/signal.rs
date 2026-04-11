// SPDX-License-Identifier: MPL-2.0

use loongArch64::register::estat::Exception;
use ostd::arch::cpu::context::{CpuExceptionInfo, UserContext};

use crate::process::signal::{
    SignalContext, constants::*, sig_num::SigNum, signals::fault::FaultSignal,
};

impl SignalContext for UserContext {
    fn set_arguments(&mut self, sig_num: SigNum, siginfo_addr: usize, ucontext_addr: usize) {
        self.set_a0(sig_num.as_u8() as usize);
        self.set_a1(siginfo_addr);
        self.set_a2(ucontext_addr);
    }
}

impl From<&CpuExceptionInfo> for FaultSignal {
    fn from(trap_info: &CpuExceptionInfo) -> Self {
        let (num, code, addr) = match trap_info.code {
            Exception::LoadPageFault | Exception::StorePageFault | Exception::FetchPageFault => {
                (SIGSEGV, SEGV_MAPERR, Some(trap_info.page_fault_addr as u64))
            }
            Exception::PageModifyFault
            | Exception::PageNonReadableFault
            | Exception::PageNonExecutableFault
            | Exception::PagePrivilegeIllegal => {
                (SIGSEGV, SEGV_ACCERR, Some(trap_info.page_fault_addr as u64))
            }
            Exception::FetchInstructionAddressError | Exception::MemoryAccessAddressError => {
                (SIGBUS, BUS_ADRERR, None)
            }
            Exception::AddressNotAligned => (SIGBUS, BUS_ADRALN, None),
            Exception::BoundsCheckFault => (SIGSEGV, SEGV_BNDERR, None),
            Exception::Breakpoint => (SIGTRAP, TRAP_BRKPT, None),
            Exception::InstructionNotExist | Exception::InstructionPrivilegeIllegal => {
                (SIGILL, ILL_ILLOPC, None)
            }
            Exception::FloatingPointUnavailable => (SIGFPE, FPE_FLTINV, None),
            Exception::Syscall | Exception::TLBRFill => unreachable!(),
        };

        FaultSignal::new(num, code, addr)
    }
}
