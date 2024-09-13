// SPDX-License-Identifier: MPL-2.0

use core::{sync::atomic::Ordering, time::Duration};

use ostd::sync::{WaitQueue, Waiter};

use super::sig_mask::SigMask;
use crate::{
    prelude::*, process::posix_thread::PosixThreadExt, thread::Thread, time::wait::WaitTimeout,
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
    /// Pauses the execution of the current thread until the `cond` is met ( i.e., `cond()`
    /// returns `Some(_)` ), or some signals are received by the current thread or process.
    ///
    /// # Errors
    ///
    /// If some signals are received before `cond` is met, this method will return `Err(EINTR)`.
    ///
    /// [`EINTR`]: crate::error::Errno::EINTR
    fn pause_until<F, R>(&self, cond: F) -> Result<R>
    where
        F: FnMut() -> Option<R>,
    {
        self.pause_until_or_timeout_opt(cond, None)
    }

    /// Pauses the execution of the current thread until the `cond` is met ( i.e., `cond()`
    /// returns `Some(_)` ), or some signals are received by the current thread or process,
    /// or the given `timeout` is expired.
    ///
    /// # Errors
    ///
    /// If `timeout` is expired before the `cond` is met or some signals are received,
    /// this method will return `Err(ETIME)`. If the pausing is interrupted by some signals,
    /// this method will return `Err(EINTR)`
    ///
    /// [`ETIME`]: crate::error::Errno::ETIME
    /// [`EINTR`]: crate::error::Errno::EINTR
    fn pause_until_or_timeout<F, R>(&self, mut cond: F, timeout: &Duration) -> Result<R>
    where
        F: FnMut() -> Option<R>,
    {
        if *timeout == Duration::ZERO {
            return cond()
                .ok_or_else(|| Error::with_message(Errno::ETIME, "the time limit is reached"));
        }
        self.pause_until_or_timeout_opt(cond, Some(timeout))
    }

    /// Pauses the execution of the current thread until the `cond` is met ( i.e., `cond()`
    /// returns `Some(_)` ), or some signals are received by the current thread or process.
    /// If the input `timeout` is set, the pausing will finish when the `timeout` is expired.
    ///
    /// # Errors
    ///
    /// If `timeout` is expired before the `cond` is met or some signals are received,
    /// this method will return `Err(ETIME)`. If the pausing is interrupted by some signals,
    /// this method will return `Err(EINTR)`
    ///
    /// [`ETIME`]: crate::error::Errno::ETIME
    /// [`EINTR`]: crate::error::Errno::EINTR
    #[doc(hidden)]
    fn pause_until_or_timeout_opt<F, R>(&self, cond: F, timeout: Option<&Duration>) -> Result<R>
    where
        F: FnMut() -> Option<R>;
}

impl Pause for Waiter {
    fn pause_until_or_timeout_opt<F, R>(&self, mut cond: F, timeout: Option<&Duration>) -> Result<R>
    where
        F: FnMut() -> Option<R>,
    {
        if let Some(res) = cond() {
            return Ok(res);
        }

        let current_thread = self.task().data().downcast_ref::<Arc<Thread>>();

        let Some(posix_thread) = current_thread.and_then(|thread| thread.as_posix_thread()) else {
            if let Some(timeout) = timeout {
                return self.wait_until_or_timeout(cond, timeout);
            } else {
                return self.wait_until_or_cancelled(cond, || Ok(()));
            }
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
        let res = if let Some(timeout) = timeout {
            self.wait_until_or_timeout_cancelled(cond, cancel_cond, timeout)
        } else {
            self.wait_until_or_cancelled(cond, cancel_cond)
        };
        posix_thread.clear_signalled_waker();
        res
    }
}

impl Pause for WaitQueue {
    fn pause_until_or_timeout_opt<F, R>(&self, mut cond: F, timeout: Option<&Duration>) -> Result<R>
    where
        F: FnMut() -> Option<R>,
    {
        if let Some(res) = cond() {
            return Ok(res);
        }

        let (waiter, _) = Waiter::new_pair();
        let cond = || {
            self.enqueue(waiter.waker());
            cond()
        };
        waiter.pause_until_or_timeout_opt(cond, timeout)
    }
}

/// Executes a closure while temporarily blocking some signals for the current POSIX thread.
pub fn with_signal_blocked<R>(ctx: &Context, mask: SigMask, operate: impl FnOnce() -> R) -> R {
    let posix_thread = ctx.posix_thread;
    let sig_mask = posix_thread.sig_mask();

    let old_mask = sig_mask.load(Ordering::Relaxed);
    sig_mask.store(old_mask + mask, Ordering::Relaxed);

    let res = operate();

    sig_mask.store(old_mask, Ordering::Relaxed);

    res
}

#[cfg(ktest)]
mod test {
    use core::sync::atomic::AtomicBool;

    use ostd::prelude::*;

    use super::*;
    use crate::thread::{
        kernel_thread::{KernelThreadExt, ThreadOptions},
        Thread,
    };

    #[ktest]
    fn test_waiter_pause() {
        let wait_queue = Arc::new(WaitQueue::new());
        let wait_queue_cloned = wait_queue.clone();

        let boolean = Arc::new(AtomicBool::new(false));
        let boolean_cloned = boolean.clone();

        let thread = Thread::spawn_kernel_thread(ThreadOptions::new(move || {
            Thread::yield_now();

            boolean_cloned.store(true, Ordering::Relaxed);
            wait_queue_cloned.wake_all();
        }));

        wait_queue
            .pause_until(|| boolean.load(Ordering::Relaxed).then_some(()))
            .unwrap();

        thread.join();
    }
}
