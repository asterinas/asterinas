// SPDX-License-Identifier: MPL-2.0

use ostd::cpu::{CpuException, CpuExceptionInfo};

use super::Signal;
use crate::{
    prelude::*,
    process::signal::{c_types::siginfo_t, constants::*, sig_num::SigNum},
};
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaultSignal {
    num: SigNum,
    code: i32,
    addr: Option<u64>,
}

impl FaultSignal {
    pub fn new(trap_info: &CpuExceptionInfo) -> FaultSignal {
        debug!("Trap id: {}", trap_info.id);
        let exception = CpuException::from_num(trap_info.id as u16);
        let (num, code, addr) = match exception {
            CpuException::DivideByZero => (SIGFPE, FPE_INTDIV, None),
            CpuException::X87FloatingPointException | CpuException::SimdFloatingPointException => {
                (SIGFPE, FPE_FLTDIV, None)
            }
            CpuException::BoundRangeExceeded => (SIGSEGV, SEGV_BNDERR, None),
            CpuException::AlignmentCheck => (SIGBUS, BUS_ADRALN, None),
            CpuException::InvalidOpcode => (SIGILL, ILL_ILLOPC, None),
            CpuException::GeneralProtectionFault => (SIGBUS, BUS_ADRERR, None),
            CpuException::PageFault => {
                const PF_ERR_FLAG_PRESENT: usize = 1usize << 0;
                let code = if trap_info.error_code & PF_ERR_FLAG_PRESENT != 0 {
                    SEGV_ACCERR
                } else {
                    SEGV_MAPERR
                };
                let addr = Some(trap_info.page_fault_addr as u64);
                (SIGSEGV, code, addr)
            }
            _ => panic!("Exception cannnot be a signal"),
        };
        FaultSignal { num, code, addr }
    }
}

impl Signal for FaultSignal {
    fn num(&self) -> SigNum {
        self.num
    }

    fn to_info(&self) -> siginfo_t {
        siginfo_t::new(self.num, self.code)
        // info.set_si_addr(self.addr.unwrap_or_default() as *const c_void);
        // info
    }
}
