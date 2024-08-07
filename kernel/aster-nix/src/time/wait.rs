// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use ostd::sync::{WaitQueue, Waiter, Waker};

use super::{clocks::JIFFIES_TIMER_MANAGER, timer::Timeout, Timer, TimerManager};
use crate::prelude::*;

/// A trait that provide the timeout related function for [`WaitQueue`]`.
pub trait WaitTimeout {
    /// Wait until some condition returns `Some(_)`, or a given timeout is reached. If
    /// the condition does not becomes `Some(_)` before the timeout is reached, the
    /// function will return `None`.
    ///
    /// The timeout is against the default JIFFIES clock.
    fn wait_until_or_timeout<F, R>(&self, cond: F, timeout: &Duration) -> Option<R>
    where
        F: FnMut() -> Option<R>,
    {
        let timeout_against_clock = WakerTimerCreater::new(timeout);
        self.wait_until_or_timeout_against_clock(cond, &timeout_against_clock)
    }

    /// Similar to [`WaitTimeout::wait_until_or_timeout`].
    ///
    /// The difference is that the timeout of this method is against the specified clock,
    /// which is defined in `timer_creater`.
    fn wait_until_or_timeout_against_clock<F, R>(
        &self,
        cond: F,
        timer_creater: &WakerTimerCreater,
    ) -> Option<R>
    where
        F: FnMut() -> Option<R>;
}

/// A struct for creating [`Timer`] against a specific timer manager for [`Waker`].
pub struct WakerTimerCreater<'a> {
    timeout: Timeout,
    timer_manager: &'a Arc<TimerManager>,
}

impl<'a> WakerTimerCreater<'a> {
    /// Creates a new `WakerTimerCreater` against the default JIFFIES clock.
    pub fn new(timeout: &Duration) -> Self {
        let timeout = Timeout::After(*timeout);
        let jiffies_timer_manager = JIFFIES_TIMER_MANAGER.get().unwrap();
        Self::new_with_timer_manager(timeout, jiffies_timer_manager)
    }

    /// Creates a new `WakerTimerCreater` with given timer manager.
    pub const fn new_with_timer_manager(
        timeout: Timeout,
        timer_manager: &'a Arc<TimerManager>,
    ) -> Self {
        Self {
            timeout,
            timer_manager,
        }
    }

    /// Returns the timeout
    pub const fn timeout(&self) -> &Timeout {
        &self.timeout
    }

    fn is_expired(&self) -> bool {
        self.timer_manager.is_expired_timeout(&self.timeout)
    }

    fn create_waker_timer(&self, waker: Arc<Waker>) -> Arc<Timer> {
        let timer = self.timer_manager.create_timer(move || {
            waker.wake_up();
        });
        timer.set_timeout(self.timeout.clone());
        timer
    }
}

impl WaitTimeout for WaitQueue {
    fn wait_until_or_timeout_against_clock<F, R>(
        &self,
        mut cond: F,
        timer_creater: &WakerTimerCreater,
    ) -> Option<R>
    where
        F: FnMut() -> Option<R>,
    {
        if timer_creater.is_expired() {
            return cond();
        }

        if let Some(res) = cond() {
            return Some(res);
        }

        let (waiter, waker) = Waiter::new_pair();

        let timer = timer_creater.create_waker_timer(waker);

        let cancel_cond = {
            let timer = timer.clone();
            move || timer.remain() == Duration::ZERO
        };
        let res = self.wait_until_or_cancelled(cond, waiter, cancel_cond);

        // If res is `Some`, then the timeout may not have been expired. We cancel it manually.
        if res.is_some() {
            timer.cancel();
        }

        res
    }
}
