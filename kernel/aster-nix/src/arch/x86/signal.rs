// SPDX-License-Identifier: MPL-2.0

use ostd::cpu::UserContext;

use crate::process::signal::{sig_num::SigNum, SignalContext};

impl SignalContext for UserContext {
    fn set_arguments(&mut self, sig_num: SigNum, siginfo_addr: usize, ucontext_addr: usize) {
        self.set_rdi(sig_num.as_u8() as usize);
        self.set_rsi(siginfo_addr);
        self.set_rdx(ucontext_addr);
    }
}
