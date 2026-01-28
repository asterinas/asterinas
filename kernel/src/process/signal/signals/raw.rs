// SPDX-License-Identifier: MPL-2.0

use core::fmt::Debug;

use crate::process::signal::{c_types::siginfo_t, sig_num::SigNum, signals::Signal};

#[derive(Clone, Copy)]
pub struct RawSignal {
    info: siginfo_t,
}

impl Debug for RawSignal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RawSignal")
            .field("signo", &self.info.si_signo)
            .field("errno", &self.info.si_errno)
            .field("code", &self.info.si_code)
            .finish()
    }
}

impl RawSignal {
    // Caller must ensure the si_signo is valid.
    pub fn new(info: siginfo_t) -> Self {
        Self { info }
    }
}

impl Signal for RawSignal {
    fn num(&self) -> SigNum {
        SigNum::from_u8(self.info.si_signo as u8)
    }

    fn to_info(&self) -> siginfo_t {
        self.info
    }
}
