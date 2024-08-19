// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use ostd::sync::{WaitQueue, Waiter};

use super::{clocks::JIFFIES_TIMER_MANAGER, timer::Timeout};

/// A trait that provide the timeout related function for [`WaitQueue`]`.
pub trait WaitTimeout {
    /// Wait until some condition returns `Some(_)`, or a given timeout is reached. If
    /// the condition does not becomes `Some(_)` before the timeout is reached, the
    /// function will return `None`.
    fn wait_until_or_timeout<F, R>(&self, cond: F, timeout: &Duration) -> Option<R>
    where
        F: FnMut() -> Option<R>;
}

impl WaitTimeout for WaitQueue {
    fn wait_until_or_timeout<F, R>(&self, mut cond: F, timeout: &Duration) -> Option<R>
    where
        F: FnMut() -> Option<R>,
    {
        if *timeout == Duration::ZERO {
            return cond();
        }

        if let Some(res) = cond() {
            return Some(res);
        }

        let (waiter, waker) = Waiter::new_pair();

        let jiffies_timer = JIFFIES_TIMER_MANAGER.get().unwrap().create_timer(move || {
            waker.wake_up();
        });
        jiffies_timer.set_timeout(Timeout::After(*timeout));

        let cancel_cond = {
            let jiffies_timer = jiffies_timer.clone();
            move || jiffies_timer.remain() == Duration::ZERO
        };
        let res = self.wait_until_or_cancelled(cond, waiter, cancel_cond);

        // If res is `Some`, then the timeout may not have been expired. We cancel it manually.
        if res.is_some() {
            jiffies_timer.cancel();
        }

        res
    }
}
