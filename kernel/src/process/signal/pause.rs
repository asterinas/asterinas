// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use ostd::sync::{WaitQueue, Waiter};

use super::sig_mask::SigMask;
use crate::{
    prelude::*,
    process::posix_thread::AsPosixThread,
    thread::AsThread,
    time::wait::{ManagedTimeout, TimeoutExt},
};

/// `Pause` is an extension trait to make [`Waiter`] and [`WaitQueue`] signal aware.
///
/// The original methods of `Waiter` and `WaitQueue` only allow a thread
/// to wait (via `wait`) until it is woken up (via `wake_up`, `wake_one`, or `wake_all`)
/// or a condition is met (via `wait_until`).
/// The `WaitTimeout` extension trait grants the extra ability
/// to wait until a timeout (via `wait_until_or_timeout`).
/// On top of `WaitTimeout`, this `Pause` trait provides the `pause`-family methods,
/// which are similar to the `wait`-family methods except that the methods also return
/// when the waiting thread is interrupted by a POSIX signal.
/// When this happens, the `pause`-family methods return `Err(EINTR)`.
pub trait Pause: WaitTimeout {
    /// Pauses until the condition is met or a signal interrupts.
    ///
    /// # Errors
    ///
    /// This method will return an error with [`EINTR`] if a signal is received before the
    /// condition is met.
    ///
    /// [`EINTR`]: crate::error::Errno::EINTR
    #[track_caller]
    fn pause_until<F, R>(&self, cond: F) -> Result<R>
    where
        F: FnMut() -> Option<R>,
    {
        self.pause_until_or_timeout_impl(cond, None)
    }

    /// Pauses until the condition is met, the timeout is reached, or a signal interrupts.
    ///
    /// # Errors
    ///
    /// Before the condition is met, this method will return an error with
    ///  - [`EINTR`] if a signal is received;
    ///  - [`ETIME`] if the timeout is reached.
    ///
    /// [`ETIME`]: crate::error::Errno::ETIME
    /// [`EINTR`]: crate::error::Errno::EINTR
    #[track_caller]
    fn pause_until_or_timeout<'a, F, T, R>(&self, mut cond: F, timeout: T) -> Result<R>
    where
        F: FnMut() -> Option<R>,
        T: Into<TimeoutExt<'a>>,
    {
        let timeout = timeout.into();
        let timeout_inner = match timeout.check_expired() {
            Ok(inner) => inner,
            Err(err) => return cond().ok_or(err),
        };

        self.pause_until_or_timeout_impl(cond, timeout_inner)
    }

    /// Pauses until the condition is met, the timeout is reached, or a signal interrupts.
    ///
    /// # Errors
    ///
    /// Before the condition is met, this method will return an error with
    ///  - [`EINTR`] if a signal is received;
    ///  - [`ETIME`] if the timeout is reached.
    ///
    /// [`ETIME`]: crate::error::Errno::ETIME
    /// [`EINTR`]: crate::error::Errno::EINTR
    #[doc(hidden)]
    #[track_caller]
    fn pause_until_or_timeout_impl<F, R>(
        &self,
        cond: F,
        timeout: Option<&ManagedTimeout>,
    ) -> Result<R>
    where
        F: FnMut() -> Option<R>;

    /// Pauses until the thread is woken up, the timeout is reached, or a signal interrupts.
    ///
    /// # Errors
    ///
    /// This method will return an error with
    ///  - [`EINTR`] if a signal is received;
    ///  - [`ETIME`] if the timeout is reached.
    ///
    /// [`ETIME`]: crate::error::Errno::ETIME
    /// [`EINTR`]: crate::error::Errno::EINTR
    #[track_caller]
    fn pause_timeout(&self, timeout: &TimeoutExt<'_>) -> Result<()>;
}

impl Pause for Waiter {
    fn pause_until_or_timeout_impl<F, R>(
        &self,
        cond: F,
        timeout: Option<&ManagedTimeout>,
    ) -> Result<R>
    where
        F: FnMut() -> Option<R>,
    {
        // No fast paths for `Waiter`. If the caller wants a fast path, it should do so _before_
        // the waiter is created.

        let Some(posix_thread) = self
            .task()
            .as_thread()
            .and_then(|thread| thread.as_posix_thread())
        else {
            return self.wait_until_or_timeout_cancelled(cond, || Ok(()), timeout);
        };

        let cancel_cond = || {
            if posix_thread.has_pending() {
                return Err(Error::with_message(
                    Errno::EINTR,
                    "the current thread is interrupted by a signal",
                ));
            }
            Ok(())
        };

        posix_thread.set_signalled_waker(self.waker());
        let res = self.wait_until_or_timeout_cancelled(cond, cancel_cond, timeout);
        posix_thread.clear_signalled_waker();

        res
    }

    fn pause_timeout(&self, timeout: &TimeoutExt<'_>) -> Result<()> {
        let timer = timeout.check_expired()?.map(|timeout| {
            let waker = self.waker();
            timeout.create_timer(move || {
                waker.wake_up();
            })
        });

        let posix_thread_opt = self
            .task()
            .as_thread()
            .and_then(|thread| thread.as_posix_thread());

        if let Some(posix_thread) = posix_thread_opt {
            posix_thread.set_signalled_waker(self.waker());
            self.wait();
            posix_thread.clear_signalled_waker();
        } else {
            self.wait();
        }

        if let Some(timer) = timer {
            if timer.remain().is_zero() {
                return_errno_with_message!(Errno::ETIME, "the time limit is reached");
            }
            // If the timeout is not expired, cancel the timer manually.
            timer.cancel();
        }

        if posix_thread_opt
            .as_ref()
            .is_some_and(|posix_thread| posix_thread.has_pending())
        {
            return_errno_with_message!(
                Errno::EINTR,
                "the current thread is interrupted by a signal"
            );
        }

        Ok(())
    }
}

impl Pause for WaitQueue {
    fn pause_until_or_timeout_impl<F, R>(
        &self,
        mut cond: F,
        timeout: Option<&ManagedTimeout>,
    ) -> Result<R>
    where
        F: FnMut() -> Option<R>,
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
        waiter.pause_until_or_timeout_impl(cond, timeout)
    }

    fn pause_timeout(&self, _timeout: &TimeoutExt<'_>) -> Result<()> {
        panic!("`pause_timeout` can only be used on `Waiter`");
    }
}

/// Executes a closure after temporarily adjusting the signal mask of the current POSIX thread.
pub fn with_sigmask_changed<R>(
    ctx: &Context,
    mask_op: impl FnOnce(SigMask) -> SigMask,
    operate: impl FnOnce() -> R,
) -> R {
    let sig_mask = ctx.posix_thread.sig_mask();

    // Save the original signal mask and apply the mask updates.
    let old_mask = sig_mask.load(Ordering::Relaxed);
    sig_mask.store(mask_op(old_mask), Ordering::Relaxed);

    // Perform the operation.
    let res = operate();

    // Restore the original signal mask.
    sig_mask.store(old_mask, Ordering::Relaxed);

    res
}

#[cfg(ktest)]
mod test {
    use core::sync::atomic::AtomicBool;

    use ostd::prelude::*;

    use super::*;
    use crate::thread::{kernel_thread::ThreadOptions, Thread};

    #[ktest]
    fn test_waiter_pause() {
        let wait_queue = Arc::new(WaitQueue::new());
        let wait_queue_cloned = wait_queue.clone();

        let boolean = Arc::new(AtomicBool::new(false));
        let boolean_cloned = boolean.clone();

        let thread = ThreadOptions::new(move || {
            Thread::yield_now();

            boolean_cloned.store(true, Ordering::Relaxed);
            wait_queue_cloned.wake_all();
        })
        .spawn();

        wait_queue
            .pause_until(|| boolean.load(Ordering::Relaxed).then_some(()))
            .unwrap();

        thread.join();
    }
}
