// SPDX-License-Identifier: MPL-2.0

use ostd::cpu::{CpuException, CpuExceptionInfo, UserContext};

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

impl From<&CpuExceptionInfo> for FaultSignal {
    fn from(trap_info: &CpuExceptionInfo) -> Self {
        let exception = CpuException::to_cpu_exception(trap_info.id as u16).unwrap();
        let (num, code, addr) = match exception {
            CpuException::DIVIDE_BY_ZERO => (SIGFPE, FPE_INTDIV, None),
            CpuException::X87_FLOATING_POINT_EXCEPTION
            | CpuException::SIMD_FLOATING_POINT_EXCEPTION => (SIGFPE, FPE_FLTDIV, None),
            CpuException::BOUND_RANGE_EXCEEDED => (SIGSEGV, SEGV_BNDERR, None),
            CpuException::ALIGNMENT_CHECK => (SIGBUS, BUS_ADRALN, None),
            CpuException::INVALID_OPCODE => (SIGILL, ILL_ILLOPC, None),
            CpuException::GENERAL_PROTECTION_FAULT => (SIGBUS, BUS_ADRERR, None),
            CpuException::PAGE_FAULT => {
                const PF_ERR_FLAG_PRESENT: usize = 1usize << 0;
                let code = if trap_info.error_code & PF_ERR_FLAG_PRESENT != 0 {
                    SEGV_ACCERR
                } else {
                    SEGV_MAPERR
                };
                let addr = Some(trap_info.page_fault_addr as u64);
                (SIGSEGV, code, addr)
            }
            _ => panic!("Exception cannot be a signal"),
        };
        FaultSignal::new(num, code, addr)
    }
}
