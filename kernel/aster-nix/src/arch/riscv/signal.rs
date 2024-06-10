// SPDX-License-Identifier: MPL-2.0

use aster_frame::cpu::UserContext;

use crate::process::signal::{sig_num::SigNum, SignalContext};

impl SignalContext for UserContext {
    fn set_arguments(&mut self, sig_num: SigNum, siginfo_addr: usize, ucontext_addr: usize) {
        self.set_a0(sig_num.as_u8() as usize);
        self.set_a1(siginfo_addr);
        self.set_a2(ucontext_addr);
    }
}
