// SPDX-License-Identifier: MPL-2.0

use super::{constants::*, sig_action::SigAction, sig_num::SigNum};

#[derive(Copy, Clone)]
pub struct SigDispositions {
    // SigNum -> SigAction
    map: [SigAction; COUNT_ALL_SIGS],
}

impl Default for SigDispositions {
    fn default() -> Self {
        Self::new()
    }
}

impl SigDispositions {
    pub fn new() -> Self {
        Self {
            map: [SigAction::default(); COUNT_ALL_SIGS],
        }
    }

    pub fn get(&self, num: SigNum) -> SigAction {
        let idx = Self::num_to_idx(num);
        self.map[idx]
    }

    pub fn set(&mut self, num: SigNum, sa: SigAction) -> SigAction {
        let idx = Self::num_to_idx(num);
        core::mem::replace(&mut self.map[idx], sa)
    }

    pub fn set_default(&mut self, num: SigNum) {
        let idx = Self::num_to_idx(num);
        self.map[idx] = SigAction::Dfl;
    }

    /// man 7 signal:
    /// When execve, the handled signals are reset to the default; the dispositions of
    /// ignored signals are left unchanged.
    /// This function should be used when execve.
    pub fn inherit(&mut self) {
        for sigaction in &mut self.map {
            if let SigAction::User { .. } = sigaction {
                *sigaction = SigAction::Dfl;
            }
        }
    }

    fn num_to_idx(num: SigNum) -> usize {
        (num.as_u8() - MIN_STD_SIG_NUM) as usize
    }
}
