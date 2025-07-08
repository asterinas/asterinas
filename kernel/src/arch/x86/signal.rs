// SPDX-License-Identifier: MPL-2.0

use ostd::cpu::context::{CpuException, PageFaultErrorCode, UserContext};

use crate::process::signal::{
    constants::*, sig_num::SigNum, signals::fault::FaultSignal, SignalContext,
};

impl SignalContext for UserContext {
    fn set_arguments(&mut self, sig_num: SigNum, siginfo_addr: usize, ucontext_addr: usize) {
        self.set_rdi(sig_num.as_u8() as usize);
        self.set_rsi(siginfo_addr);
        self.set_rdx(ucontext_addr);
    }
}

impl From<&CpuException> for FaultSignal {
    fn from(exception: &CpuException) -> Self {
        let (num, code, addr) = match exception {
            CpuException::DivisionError => (SIGFPE, FPE_INTDIV, None),
            CpuException::X87FloatingPointException | CpuException::SIMDFloatingPointException => {
                (SIGFPE, FPE_FLTDIV, None)
            }
            CpuException::BoundRangeExceeded => (SIGSEGV, SEGV_BNDERR, None),
            CpuException::AlignmentCheck => (SIGBUS, BUS_ADRALN, None),
            CpuException::InvalidOpcode => (SIGILL, ILL_ILLOPC, None),
            CpuException::GeneralProtectionFault(..) => (SIGBUS, BUS_ADRERR, None),
            CpuException::PageFault(raw_page_fault_info) => {
                let code = if raw_page_fault_info
                    .error_code
                    .contains(PageFaultErrorCode::PRESENT)
                {
                    SEGV_ACCERR
                } else {
                    SEGV_MAPERR
                };
                let addr = Some(raw_page_fault_info.addr as u64);
                (SIGSEGV, code, addr)
            }
            e => panic!("{e:?} cannot be handled via signals ({exception:?})"),
        };

        FaultSignal::new(num, code, addr)
    }
}
