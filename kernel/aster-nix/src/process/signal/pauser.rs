// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use core::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use ostd::{sync::WaitQueue, task::Task};

use super::{sig_mask::SigSet, SigEvents, SigEventsFilter};
use crate::{
    events::Observer,
    prelude::*,
    process::posix_thread::{PosixThreadExt, SharedPosixThreadInfo},
    thread::Thread,
    time::wait::WaitTimeout,
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
/// let pauser = Pauser::new(SigSet::new_full());
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
    sig_mask: SigSet,
}

impl Pauser {
    /// Creates a new `Pauser`.
    ///
    /// The `Pauser` can be interrupted by all signals
    /// except that are blocked by current thread.
    pub fn new() -> Arc<Self> {
        Self::new_with_mask(SigSet::new_empty())
    }

    /// Creates a new `Pauser` with specified `sig_mask`.
    ///
    /// The `Pauser` will ignore signals that are in `sig_mask`
    /// or blocked by current thread.
    pub fn new_with_mask(sig_mask: SigSet) -> Arc<Self> {
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
        let current_thread = Thread::current();
        let sig_queue_waiter =
            SigObserverRegistrar::new(current_thread.as_ref(), self.sig_mask, self.clone());

        let cond = || {
            if let Some(res) = cond() {
                return Some(Ok(res));
            }

            if sig_queue_waiter.is_interrupted() {
                return Some(Err(Error::with_message(
                    Errno::EINTR,
                    "the current thread is interrupted by a signal",
                )));
            }

            None
        };

        if let Some(timeout) = timeout {
            self.wait_queue
                .wait_until_or_timeout(cond, timeout)
                .ok_or_else(|| Error::with_message(Errno::ETIME, "the time limit is reached"))?
        } else {
            self.wait_queue.wait_until(cond)
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

enum SigObserverRegistrar<'a> {
    // A POSIX thread may be interrupted by a signal if the signal is not masked.
    PosixThread {
        thread: &'a SharedPosixThreadInfo,
        old_mask: SigSet,
        observer: Arc<SigQueueObserver>,
    },
    // A kernel thread ignores all signals. It is not necessary to wait for them.
    KernelThread,
}

impl<'a> SigObserverRegistrar<'a> {
    fn new(current_thread: Option<&'a Arc<Task>>, sig_mask: SigSet, pauser: Arc<Pauser>) -> Self {
        let Some(thread) = current_thread.and_then(|thread| thread.posix_thread_info()) else {
            return Self::KernelThread;
        };

        // Block `sig_mask`.
        let (old_mask, filter) = {
            let mut locked_mask = thread.sig_mask.write();

            let old_mask = *locked_mask;
            let new_mask = {
                locked_mask.block(sig_mask.as_u64());
                *locked_mask
            };

            (old_mask, SigEventsFilter::new(new_mask))
        };

        // Register `SigQueueObserver`.
        let observer = SigQueueObserver::new(pauser);
        thread.register_sigqueue_observer(Arc::downgrade(&observer) as _, filter);

        // Check pending signals after registering the observer to avoid race conditions.
        if thread.has_pending() {
            observer.set_interrupted();
        }

        Self::PosixThread {
            thread,
            old_mask,
            observer,
        }
    }

    fn is_interrupted(&self) -> bool {
        match self {
            Self::PosixThread { observer, .. } => observer.is_interrupted(),
            Self::KernelThread => false,
        }
    }
}

impl<'a> Drop for SigObserverRegistrar<'a> {
    fn drop(&mut self) {
        let Self::PosixThread {
            thread,
            old_mask,
            observer,
        } = self
        else {
            return;
        };

        // Restore the state, assuming no one else can modify the current thread's signal mask
        // during the pause.
        thread
            .sig_queues
            .unregiser_observer(&(Arc::downgrade(observer) as _));
        thread.sig_mask.write().set(old_mask.as_u64());
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

#[cfg(ktest)]
mod test {
    use ostd::{cpu::CpuSet, prelude::*, task::Priority};

    use super::*;
    use crate::thread;

    #[ktest]
    fn test_pauser() {
        let pauser = Pauser::new();
        let pauser_cloned = pauser.clone();

        let boolean = Arc::new(AtomicBool::new(false));
        let boolean_cloned = boolean.clone();

        let thread = thread::new_kernel(
            move |tctx, _, _| {
                tctx.yield_now();

                boolean_cloned.store(true, Ordering::Relaxed);
                pauser_cloned.resume_all();
            },
            Priority::normal(),
            CpuSet::new_full(),
        );

        pauser
            .pause_until(|| boolean.load(Ordering::Relaxed).then_some(()))
            .unwrap();

        thread.join();
    }
}
