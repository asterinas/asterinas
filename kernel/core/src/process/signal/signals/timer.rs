// SPDX-License-Identifier: MPL-2.0

use ostd::mm::Vaddr;

use crate::process::signal::{
    c_types::siginfo_t, constants::SI_TIMER, sig_num::SigNum, signals::Signal,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct TimerSignal {
    num: SigNum,
    timerid: i32,
    ptr: Vaddr,
}

impl TimerSignal {
    pub(crate) fn new(num: SigNum, timerid: i32, ptr: Vaddr) -> Self {
        Self { num, timerid, ptr }
    }
}

impl Signal for TimerSignal {
    fn num(&self) -> SigNum {
        self.num
    }

    fn to_info(&self) -> siginfo_t {
        let mut info = siginfo_t::new(self.num, SI_TIMER);
        // FIXME: Track the timer overrun and pass it as the second argument.
        info.set_timer(self.timerid, 0);
        info.set_ptr(self.ptr);
        info
    }
}
