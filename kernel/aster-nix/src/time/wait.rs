// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::time::Duration;

use aster_frame::sync::{WaitQueue, Waiter, Waker};

use super::clock::JIFFIES_TIMER_MANAGER;

/// A trait that provide the timeout related function for WaitQueue.
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
        if let Some(res) = cond() {
            return Some(res);
        }

        let (waiter, waker) = Waiter::new_pair();
        let wake_up = {
            let waker = waker.clone();
            move || {
                waker.wake_up();
            }
        };

        let jiffies_timer = JIFFIES_TIMER_MANAGER.get().unwrap().create_timer(wake_up);
        jiffies_timer.set_timeout(*timeout);

        loop {
            // Enqueue the waker before checking `cond()` to avoid races
            self.enqueue(waker.clone());

            if let Some(res) = cond() {
                jiffies_timer.clear();
                return Some(res);
            };

            if jiffies_timer.remain() == Duration::ZERO {
                drop(waiter);
                return cond();
            }

            waiter.wait();
        }
    }
}
