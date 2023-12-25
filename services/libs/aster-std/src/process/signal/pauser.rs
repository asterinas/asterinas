use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;

use aster_frame::sync::WaitQueue;

use crate::events::Observer;
use crate::prelude::*;
use crate::process::posix_thread::PosixThreadExt;

use super::sig_mask::SigMask;
use super::{SigEvents, SigEventsFilter};

/// A `Pauser` allows pausing the execution of the current thread until certain conditions are reached.
///
/// Behind the scene, `Pauser` is implemented with `Waiter` and `WaiterQueue`.
/// But unlike its `Waiter` relatives, `Pauser` is aware of POSIX signals:
/// if a thread paused by a `Pauser` receives a signal, then the thread will resume its execution.
///
/// Another key difference is that `Pauser` combines the two roles of `Waiter` and `WaiterQueue`
/// into one. Both putting the current thread to sleep and waking it up can be done through the
/// same `Pauser` object, using its `pause`- and `resume`-family methods.
///
/// # Example
///
/// Here is how the current thread can be put to sleep with a `Pauser`.
///
/// ```rust
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
/// ```
/// pauser.resume_all();
/// ```
pub struct Pauser {
    wait_queue: WaitQueue,
    sig_mask: SigMask,
    is_interrupted: AtomicBool,
}

impl Pauser {
    /// Create a new `Pauser`. The `Pauser` can be interrupted by all signals except that
    /// are blocked by current thread.
    pub fn new() -> Arc<Self> {
        Self::new_with_mask(SigMask::new_empty())
    }

    /// Create a new `Pauser`, the `Pauser` will ignore signals that are in `sig_mask` and
    /// blocked by current thread.
    pub fn new_with_mask(sig_mask: SigMask) -> Arc<Self> {
        let wait_queue = WaitQueue::new();
        Arc::new(Self {
            wait_queue,
            sig_mask,
            is_interrupted: AtomicBool::new(false),
        })
    }

    /// Pause the execution of current thread until the `cond` is met ( i.e., `cond()`
    /// returns `Some(_)` ), or some signal is received by current thread or process.
    ///
    /// If some signal is received before `cond` is met, this method will returns `Err(EINTR)`.
    pub fn pause_until<F, R>(self: &Arc<Self>, cond: F) -> Result<R>
    where
        F: FnMut() -> Option<R>,
    {
        self.do_pause(cond, None)
    }

    /// Pause the execution of current thread until the `cond` is met ( i.e., `cond()` returns
    /// `Some(_)` ), or some signal is received by current thread or process, or the given
    /// `timeout` is expired.
    ///
    /// If `timeout` is expired before the `cond` is met or some signal is received,
    /// it will returns `Err(ETIME)`.
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
        self.is_interrupted.store(false, Ordering::Release);

        // Register observer on sigqueue
        let observer = Arc::downgrade(self) as Weak<dyn Observer<SigEvents>>;
        let filter = {
            let sig_mask = {
                let current_thread = current_thread!();
                let poxis_thread = current_thread.as_posix_thread().unwrap();
                let mut current_sigmask = *poxis_thread.sig_mask().lock();
                current_sigmask.block(self.sig_mask.as_u64());
                current_sigmask
            };
            SigEventsFilter::new(sig_mask)
        };

        let current_thread = current_thread!();
        let posix_thread = current_thread.as_posix_thread().unwrap();
        posix_thread.register_sigqueue_observer(observer.clone(), filter);

        // Some signal may come before we register observer, so we do another check here.
        if posix_thread.has_pending_signal() {
            self.is_interrupted.store(true, Ordering::Release);
        }

        enum Res<R> {
            Ok(R),
            Interrupted,
        }

        let cond = || {
            if let Some(res) = cond() {
                return Some(Res::Ok(res));
            }

            if self.is_interrupted.load(Ordering::Acquire) {
                return Some(Res::Interrupted);
            }

            None
        };

        let res = if let Some(timeout) = timeout {
            self.wait_queue
                .wait_until_or_timeout(cond, timeout)
                .ok_or_else(|| Error::with_message(Errno::ETIME, "timeout is reached"))?
        } else {
            self.wait_queue.wait_until(cond)
        };

        posix_thread.unregiser_sigqueue_observer(&observer);

        match res {
            Res::Ok(r) => Ok(r),
            Res::Interrupted => return_errno_with_message!(Errno::EINTR, "interrupted by signal"),
        }
    }

    /// Resume all paused threads on this pauser.
    pub fn resume_all(&self) {
        self.wait_queue.wake_all();
    }

    /// Resume one paused thread on this pauser.
    pub fn resume_one(&self) {
        self.wait_queue.wake_one();
    }
}

impl Observer<SigEvents> for Pauser {
    fn on_events(&self, events: &SigEvents) {
        self.is_interrupted.store(true, Ordering::Release);
        self.wait_queue.wake_all();
    }
}
