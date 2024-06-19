// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use core::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use ostd::sync::WaitQueue;

use super::{sig_mask::SigMask, SigEvents, SigEventsFilter};
use crate::{
    events::Observer, prelude::*, process::posix_thread::PosixThreadExt, time::wait::WaitTimeout,
};

/// A `Pauser` allows pausing the execution of the current thread until certain conditions are reached.
///
/// Behind the scene, `Pauser` is implemented with [`Waiter`] and [`WaitQueue`].
/// But unlike its [`Waiter`] relatives, `Pauser` is aware of POSIX signals:
/// if a thread paused by a `Pauser` receives a signal, then the thread will resume its execution.
///
/// Another key difference is that `Pauser` combines the two roles of [`Waiter`] and [`WaitQueue`]
/// into one. Both putting the current thread to sleep and waking it up can be done through the
/// same `Pauser` object, using its `pause`- and `resume`-family methods.
///
/// [`Waiter`]: ostd::sync::Waiter
///
/// # Example
///
/// Here is how the current thread can be put to sleep with a `Pauser`.
///
/// ```no_run
/// let pauser = Pauser::new(SigMask::new_full());
/// // Pause the execution of the current thread until a user-given condition is met
/// // or the current thread is interrupted by a signal.
/// let res = pauser.pause_until(|| {
///     if cond() {
///         Some(())
///     } else {
///         None
///     }
/// });
/// match res {
///     Ok(_) => {
///         // The user-given condition is met...
///     }
///     Err(EINTR) => {
///         // A signal is received...
///     }
///     _ => unreachable!()
/// }
/// ```
///
/// Let's assume that another thread has access to the same object of `Arc<Pauser>`.
/// Then, this second thread can resume the execution of the first thread
/// even when `cond()` does not return `true`.
///
/// ```no_run
/// pauser.resume_all();
/// ```
pub struct Pauser {
    wait_queue: WaitQueue,
    sig_mask: SigMask,
}

impl Pauser {
    /// Creates a new `Pauser`.
    ///
    /// The `Pauser` can be interrupted by all signals
    /// except that are blocked by current thread.
    pub fn new() -> Arc<Self> {
        Self::new_with_mask(SigMask::new_empty())
    }

    /// Creates a new `Pauser` with specified `sig_mask`.
    ///
    /// The `Pauser` will ignore signals that are in `sig_mask`
    /// or blocked by current thread.
    pub fn new_with_mask(sig_mask: SigMask) -> Arc<Self> {
        let wait_queue = WaitQueue::new();
        Arc::new(Self {
            wait_queue,
            sig_mask,
        })
    }

    /// Pauses the execution of current thread until the `cond` is met ( i.e., `cond()`
    /// returns `Some(_)` ), or some signal is received by current thread or process.
    ///
    /// # Errors
    ///
    /// If some signal is received before `cond` is met, this method will returns `Err(EINTR)`.
    pub fn pause_until<F, R>(self: &Arc<Self>, cond: F) -> Result<R>
    where
        F: FnMut() -> Option<R>,
    {
        self.do_pause(cond, None)
    }

    /// Pauses the execution of current thread until the `cond` is met ( i.e., `cond()` returns
    /// `Some(_)` ), or some signal is received by current thread or process, or the given
    /// `timeout` is expired.
    ///
    /// # Errors
    ///
    /// If `timeout` is expired before the `cond` is met or some signal is received,
    /// it will returns [`ETIME`].
    ///
    /// [`ETIME`]: crate::error::Errno::ETIME
    pub fn pause_until_or_timeout<F, R>(self: &Arc<Self>, cond: F, timeout: &Duration) -> Result<R>
    where
        F: FnMut() -> Option<R>,
    {
        self.do_pause(cond, Some(timeout))
    }

    fn do_pause<F, R>(self: &Arc<Self>, mut cond: F, timeout: Option<&Duration>) -> Result<R>
    where
        F: FnMut() -> Option<R>,
    {
        let current_thread = current_thread!();
        let posix_thread = current_thread.as_posix_thread().unwrap();

        // Block `self.sig_mask`
        let (old_mask, filter) = {
            let mut current_mask = posix_thread.sig_mask().lock();
            let old_mask = *current_mask;

            let new_mask = {
                current_mask.block(self.sig_mask.as_u64());
                *current_mask
            };

            (old_mask, SigEventsFilter::new(new_mask))
        };

        // Register observer on sigqueue
        let observer = SigQueueObserver::new(self.clone());
        let weak_observer = Arc::downgrade(&observer) as Weak<dyn Observer<SigEvents>>;
        posix_thread.register_sigqueue_observer(weak_observer.clone(), filter);

        // Some signal may come before we register observer, so we do another check here.
        if posix_thread.has_pending() {
            observer.set_interrupted();
        }

        enum Res<R> {
            Ok(R),
            Interrupted,
        }

        let cond = {
            let cloned_observer = observer.clone();
            move || {
                if let Some(res) = cond() {
                    return Some(Res::Ok(res));
                }

                if cloned_observer.is_interrupted() {
                    return Some(Res::Interrupted);
                }

                None
            }
        };

        let res = if let Some(timeout) = timeout {
            self.wait_queue
                .wait_until_or_timeout(cond, timeout)
                .ok_or_else(|| Error::with_message(Errno::ETIME, "timeout is reached"))
        } else {
            Ok(self.wait_queue.wait_until(cond))
        };

        // Restore the state
        posix_thread.unregiser_sigqueue_observer(&weak_observer);
        posix_thread.sig_mask().lock().set(old_mask.as_u64());

        match res? {
            Res::Ok(r) => Ok(r),
            Res::Interrupted => return_errno_with_message!(Errno::EINTR, "interrupted by signal"),
        }
    }

    /// Resumes all paused threads on this pauser.
    pub fn resume_all(&self) {
        self.wait_queue.wake_all();
    }

    /// Resumes one paused thread on this pauser.
    pub fn resume_one(&self) {
        self.wait_queue.wake_one();
    }
}

struct SigQueueObserver {
    is_interrupted: AtomicBool,
    pauser: Arc<Pauser>,
}

impl SigQueueObserver {
    fn new(pauser: Arc<Pauser>) -> Arc<Self> {
        Arc::new(Self {
            is_interrupted: AtomicBool::new(false),
            pauser,
        })
    }

    fn is_interrupted(&self) -> bool {
        self.is_interrupted.load(Ordering::Acquire)
    }

    fn set_interrupted(&self) {
        self.is_interrupted.store(true, Ordering::Release);
    }
}

impl Observer<SigEvents> for SigQueueObserver {
    fn on_events(&self, _: &SigEvents) {
        self.set_interrupted();
        self.pauser.wait_queue.wake_all();
    }
}
