// SPDX-License-Identifier: MPL-2.0

use super::Signal;
use crate::{
    prelude::*,
    process::signal::{c_types::siginfo_t, sig_num::SigNum},
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaultSignal {
    num: SigNum,
    code: i32,
    addr: Option<u64>,
}

impl FaultSignal {
    pub fn new(num: SigNum, code: i32, addr: Option<u64>) -> FaultSignal {
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
