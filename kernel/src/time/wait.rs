// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use ostd::sync::{WaitQueue, Waiter};

use super::{clocks::JIFFIES_TIMER_MANAGER, timer::Timeout};
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
        self.wait_until_or_timeout_cancelled(cond, || Ok(()), timeout)
    }

    /// Waits until some condition returns `Some(_)`, or be cancelled due to
    /// reaching the timeout or the inputted cancel condition. If the condition
    /// does not becomes `Some(_)` before the timeout is reached or `cancel_cond`
    /// returns `Err`, this function will return corresponding `Err`.
    #[doc(hidden)]
    fn wait_until_or_timeout_cancelled<F, R, FCancel>(
        &self,
        cond: F,
        cancel_cond: FCancel,
        timeout: &Duration,
    ) -> Result<R>
    where
        F: FnMut() -> Option<R>,
        FCancel: Fn() -> Result<()>;
}

impl WaitTimeout for Waiter {
    fn wait_until_or_timeout_cancelled<F, R, FCancel>(
        &self,
        mut cond: F,
        cancel_cond: FCancel,
        timeout: &Duration,
    ) -> Result<R>
    where
        F: FnMut() -> Option<R>,
        FCancel: Fn() -> Result<()>,
    {
        if *timeout == Duration::ZERO {
            return cond()
                .ok_or_else(|| Error::with_message(Errno::ETIME, "the time limit is reached"));
        }

        if let Some(res) = cond() {
            return Ok(res);
        }

        let waker = self.waker();
        let jiffies_timer = JIFFIES_TIMER_MANAGER.get().unwrap().create_timer(move || {
            waker.wake_up();
        });
        jiffies_timer.set_timeout(Timeout::After(*timeout));

        let timeout_cond = {
            let jiffies_timer = jiffies_timer.clone();
            move || {
                if jiffies_timer.remain() != Duration::ZERO {
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
            jiffies_timer.cancel();
        }

        res
    }
}

impl WaitTimeout for WaitQueue {
    fn wait_until_or_timeout_cancelled<F, R, FCancel>(
        &self,
        mut cond: F,
        cancel_cond: FCancel,
        timeout: &Duration,
    ) -> Result<R>
    where
        F: FnMut() -> Option<R>,
        FCancel: Fn() -> Result<()>,
    {
        if *timeout == Duration::ZERO {
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
        waiter.wait_until_or_timeout_cancelled(cond, cancel_cond, timeout)
    }
}
