// SPDX-License-Identifier: MPL-2.0

use super::Signal;
use crate::process::signal::{c_types::siginfo_t, constants::SI_KERNEL, sig_num::SigNum};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KernelSignal {
    num: SigNum,
}

impl KernelSignal {
    pub const fn new(num: SigNum) -> Self {
        Self { num }
    }
}

impl Signal for KernelSignal {
    fn num(&self) -> SigNum {
        self.num
    }

    fn to_info(&self) -> siginfo_t {
        siginfo_t::new(self.num, SI_KERNEL)
    }
}
