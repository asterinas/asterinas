// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use ostd::sync::{WaitQueue, Waiter};

use super::{clocks::JIFFIES_TIMER_MANAGER, timer::Timeout, Timer, TimerManager};
use crate::prelude::*;

/// A trait that provide the timeout related function for [`Waiter`] and [`WaitQueue`]`.
pub trait WaitTimeout {
    /// Waits until some condition returns `Some(_)` or a given timeout is reached.
    ///
    /// # Errors
    ///
    /// If the condition does not become `Some(_)` before the timeout is reached, this function
    /// will return an error with [`ETIME`].
    ///
    /// [`ETIME`]: crate::error::Errno::ETIME
    fn wait_until_or_timeout<'a, F, T, R>(&self, mut cond: F, timeout: T) -> Result<R>
    where
        F: FnMut() -> Option<R>,
        T: Into<TimeoutExt<'a>>,
    {
        let timeout = timeout.into();
        let timeout_inner = match timeout.check_expired() {
            Ok(inner) => inner,
            Err(err) => return cond().ok_or(err),
        };

        self.wait_until_or_timeout_cancelled(cond, || Ok(()), timeout_inner)
    }

    /// Waits until some condition returns `Some(_)` or is cancelled by the timeout or cancel
    /// condition.
    ///
    /// # Errors
    ///
    /// If the condition does not become `Some(_)` before the cancellation, this function
    /// will return:
    ///  - an error with [`ETIME`] if the timeout is reached;
    ///  - the error returned by the cancel condition if the cancel condition returns `Err(_)`.
    #[doc(hidden)]
    fn wait_until_or_timeout_cancelled<F, R, FCancel>(
        &self,
        cond: F,
        cancel_cond: FCancel,
        timeout: Option<&ManagedTimeout>,
    ) -> Result<R>
    where
        F: FnMut() -> Option<R>,
        FCancel: Fn() -> Result<()>;
}

/// A timeout with extended semantics.
pub enum TimeoutExt<'a> {
    /// The timeout will never fire.
    Never,
    /// The timeout will expire later according to [`ManagedTimeout`].
    At(ManagedTimeout<'a>),
}

impl<'a> TimeoutExt<'a> {
    /// Checks whether the timeout is expired.
    ///
    /// This method will return:
    ///  - `Ok(Some(_))` if the timeout isn't expired but it may be expired later.
    ///  - `Ok(None)` if the timeout will never be expired.
    ///  - `Err(ETIME)` if the timeout is expired.
    pub fn check_expired(&self) -> Result<Option<&ManagedTimeout<'a>>> {
        match self {
            TimeoutExt::At(inner) if inner.is_expired() => {
                return_errno_with_message!(Errno::ETIME, "the time limit is reached")
            }
            TimeoutExt::At(inner) => Ok(Some(inner)),
            TimeoutExt::Never => Ok(None),
        }
    }
}

impl From<&Duration> for TimeoutExt<'_> {
    fn from(value: &Duration) -> Self {
        Self::At(ManagedTimeout::new(*value))
    }
}

impl From<Option<&Duration>> for TimeoutExt<'_> {
    fn from(value: Option<&Duration>) -> Self {
        match value {
            Some(duration) => duration.into(),
            None => Self::Never,
        }
    }
}

impl<'a> From<ManagedTimeout<'a>> for TimeoutExt<'a> {
    fn from(value: ManagedTimeout<'a>) -> Self {
        Self::At(value)
    }
}

impl<'a> From<Option<ManagedTimeout<'a>>> for TimeoutExt<'a> {
    fn from(value: Option<ManagedTimeout<'a>>) -> Self {
        match value {
            Some(timeout) => timeout.into(),
            None => Self::Never,
        }
    }
}

/// A [`Timeout`] with the associated [`TimerManager`].
pub struct ManagedTimeout<'a> {
    timeout: Timeout,
    manager: &'a Arc<TimerManager>,
}

impl<'a> ManagedTimeout<'a> {
    /// Creates a new `ManagedTimeout` with the JIFFIES timer manager.
    pub fn new(timeout: Duration) -> Self {
        let timeout = Timeout::After(timeout);
        let manager = JIFFIES_TIMER_MANAGER.get().unwrap();
        Self::new_with_manager(timeout, manager)
    }

    /// Creates a new `ManagedTimeout` with the given timer manager.
    pub const fn new_with_manager(timeout: Timeout, manager: &'a Arc<TimerManager>) -> Self {
        Self { timeout, manager }
    }

    /// Returns weather the timeout is expired.
    pub fn is_expired(&self) -> bool {
        self.manager.is_expired_timeout(&self.timeout)
    }

    /// Creates a timer for the timeout.
    pub fn create_timer<F>(&self, callback: F) -> Arc<Timer>
    where
        F: Fn() + Send + Sync + 'static,
    {
        let timer = self.manager.create_timer(callback);
        timer.set_timeout(self.timeout.clone());
        timer
    }
}

impl WaitTimeout for Waiter {
    fn wait_until_or_timeout_cancelled<F, R, FCancel>(
        &self,
        cond: F,
        cancel_cond: FCancel,
        timeout: Option<&ManagedTimeout>,
    ) -> Result<R>
    where
        F: FnMut() -> Option<R>,
        FCancel: Fn() -> Result<()>,
    {
        // No fast paths for `Waiter`. If the caller wants a fast path, it should do so _before_
        // the waiter is created.

        let timer = timeout.map(|timeout| {
            let waker = self.waker();
            timeout.create_timer(move || {
                waker.wake_up();
            })
        });

        let cancel_cond = {
            let timer = timer.clone();

            move || {
                if timer
                    .as_ref()
                    .is_some_and(|timer| timer.remain() == Duration::ZERO)
                {
                    return_errno_with_message!(Errno::ETIME, "the time limit is reached");
                }

                cancel_cond()
            }
        };

        let res = self.wait_until_or_cancelled(cond, cancel_cond);

        // If `res` is not `ETIME` error, then the timeout may not have been expired.
        // We cancel it manually.
        if let Some(timer) = timer
            && !res
                .as_ref()
                .is_err_and(|e: &Error| e.error() == Errno::ETIME)
        {
            timer.cancel();
        }

        res
    }
}

impl WaitTimeout for WaitQueue {
    fn wait_until_or_timeout_cancelled<F, R, FCancel>(
        &self,
        mut cond: F,
        cancel_cond: FCancel,
        timeout: Option<&ManagedTimeout>,
    ) -> Result<R>
    where
        F: FnMut() -> Option<R>,
        FCancel: Fn() -> Result<()>,
    {
        // Fast path:
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
