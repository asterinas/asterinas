// SPDX-License-Identifier: MPL-2.0

use super::{constants::*, sig_action::SigAction, sig_num::SigNum};
use crate::{prelude::*, process::signal::sig_action::SigActionFlags};

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

    pub fn set(&mut self, num: SigNum, sa: SigAction) -> Result<SigAction> {
        check_sigaction(&sa)?;
        let idx = Self::num_to_idx(num);
        Ok(core::mem::replace(&mut self.map[idx], sa))
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

fn check_sigaction(sig_action: &SigAction) -> Result<()> {
    // Here we only check if the SA_RESTORER flag is set and restorer_addr is not 0.
    // Note: Linux checks the SA_RESTORER flag when initializing the signal stack,
    // whereas we have moved this check forward to prevent this action from being set.
    // This may result in some differences from the behavior of Linux.

    let SigAction::User {
        flags,
        restorer_addr,
        ..
    } = sig_action
    else {
        return Ok(());
    };

    if flags.contains(SigActionFlags::SA_RESTORER) && *restorer_addr != 0 {
        return Ok(());
    }

    cfg_if::cfg_if! {
        if #[cfg(target_arch = "x86_64")] {
            // On x86-64, `SA_RESTORER` is mandatory and cannot be omitted.
            // Ref: <https://elixir.bootlin.com/linux/v6.13/source/arch/x86/kernel/signal_64.c#L172>
            return_errno_with_message!(Errno::EINVAL, "x86-64 should always use SA_RESTORER");
        } else {
            // FIXME: The support for user-provided signal handlers
            // without `SA_RESTORER` is arch-dependent.
            // Other archs may need to handle scenarios where `SA_RESTORER` is omitted.
            return_errno_with_message!(Errno::EINVAL, "TODO: properly deal with SA_RESTORER");
        }
    }
}
