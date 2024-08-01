// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use ostd::sync::{WaitQueue, Waiter};

use super::{clocks::JIFFIES_TIMER_MANAGER, timer::Timeout, Timer, TimerManager};
use crate::prelude::*;

/// A trait that provide the timeout related function for [`Waiter`] and [`WaitQueue`]`.
pub trait WaitTimeout {
    /// Waits until some condition returns `Some(_)`, or a given timeout is reached. If
    /// the condition does not becomes `Some(_)` before the timeout is reached,
    /// this function will return `ETIME` error.
    fn wait_until_or_timeout<F, R>(&self, cond: F, timeout: &Duration) -> Result<R>
    where
        F: FnMut() -> Option<R>,
    {
        let timer_builder = TimerBuilder::new(timeout);
        self.wait_until_or_timer_timeout(cond, &timer_builder)
    }

    /// Similar to [`WaitTimeout::wait_until_or_timeout`].
    ///
    /// The difference is that the timeout of this method is against the specified clock,
    /// which is defined in `timer_builder`.
    fn wait_until_or_timer_timeout<F, R>(&self, cond: F, timer_builder: &TimerBuilder) -> Result<R>
    where
        F: FnMut() -> Option<R>,
    {
        self.wait_until_or_timer_timeout_cancelled(cond, || Ok(()), timer_builder)
    }

    /// Waits until some condition returns `Some(_)`, or be cancelled due to
    /// reaching the timeout or the inputted cancel condition. If the condition
    /// does not becomes `Some(_)` before the timeout is reached or `cancel_cond`
    /// returns `Err`, this function will return corresponding `Err`.
    #[doc(hidden)]
    fn wait_until_or_timer_timeout_cancelled<F, R, FCancel>(
        &self,
        cond: F,
        cancel_cond: FCancel,
        timer_builder: &TimerBuilder,
    ) -> Result<R>
    where
        F: FnMut() -> Option<R>,
        FCancel: Fn() -> Result<()>;
}

/// A helper structure to build timers from a specified timeout and timer manager.
pub struct TimerBuilder<'a> {
    timeout: Timeout,
    timer_manager: &'a Arc<TimerManager>,
}

impl<'a> TimerBuilder<'a> {
    /// Creates a new `TimerBuilder` against the default JIFFIES clock.
    pub fn new(timeout: &Duration) -> Self {
        let timeout = Timeout::After(*timeout);
        let jiffies_timer_manager = JIFFIES_TIMER_MANAGER.get().unwrap();
        Self::new_with_timer_manager(timeout, jiffies_timer_manager)
    }

    /// Creates a new `TimerBuilder` with given timer manager.
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

    /// Builds and sets a timer,
    /// which will trigger `callback` when `self.timeout()` is reached.
    pub fn fire<F>(&self, callback: F) -> Arc<Timer>
    where
        F: Fn() + Send + Sync + 'static,
    {
        let timer = self.timer_manager.create_timer(callback);
        timer.set_timeout(self.timeout.clone());
        timer
    }
}

impl WaitTimeout for Waiter {
    fn wait_until_or_timer_timeout_cancelled<F, R, FCancel>(
        &self,
        mut cond: F,
        cancel_cond: FCancel,
        timer_builder: &TimerBuilder,
    ) -> Result<R>
    where
        F: FnMut() -> Option<R>,
        FCancel: Fn() -> Result<()>,
    {
        if timer_builder.is_expired() {
            return cond()
                .ok_or_else(|| Error::with_message(Errno::ETIME, "the time limit is reached"));
        }

        if let Some(res) = cond() {
            return Ok(res);
        }

        let waker = self.waker();
        let timer = timer_builder.fire(move || {
            waker.wake_up();
        });

        let timeout_cond = {
            let timer = timer.clone();
            move || {
                if timer.remain() != Duration::ZERO {
                    Ok(())
                } else {
                    Err(Error::with_message(
                        Errno::ETIME,
                        "the time limit is reached",
                    ))
                }
            }
        };

        let cancel_cond = || {
            timeout_cond()?;
            cancel_cond()
        };

        let res = self.wait_until_or_cancelled(cond, cancel_cond);

        // If `res` is not `ETIME` error, then the timeout may not have been expired.
        // We cancel it manually.
        if !res
            .as_ref()
            .is_err_and(|e: &Error| e.error() == Errno::ETIME)
        {
            timer.cancel();
        }

        res
    }
}

impl WaitTimeout for WaitQueue {
    fn wait_until_or_timer_timeout_cancelled<F, R, FCancel>(
        &self,
        mut cond: F,
        cancel_cond: FCancel,
        timer_builder: &TimerBuilder,
    ) -> Result<R>
    where
        F: FnMut() -> Option<R>,
        FCancel: Fn() -> Result<()>,
    {
        if timer_builder.is_expired() {
            return cond()
                .ok_or_else(|| Error::with_message(Errno::ETIME, "the time limit is reached"));
        }

        if let Some(res) = cond() {
            return Ok(res);
        }

        let (waiter, _) = Waiter::new_pair();
        let cond = || {
            self.enqueue(waiter.waker());
            cond()
        };
        waiter.wait_until_or_timer_timeout_cancelled(cond, cancel_cond, timer_builder)
    }
}
