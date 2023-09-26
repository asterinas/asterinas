use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;

use jinux_frame::sync::WaitQueue;

use crate::events::{Events, EventsFilter, Observer};
use crate::prelude::*;
use crate::process::posix_thread::PosixThreadExt;

use super::sig_mask::SigMask;
use super::sig_num::SigNum;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SigEvents(SigNum);

impl SigEvents {
    pub fn new(sig_num: SigNum) -> Self {
        Self(sig_num)
    }
}

impl Events for SigEvents {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SigEventsFilter(SigMask);

impl SigEventsFilter {
    pub fn new(mask: SigMask) -> Self {
        Self(mask)
    }
}

impl EventsFilter<SigEvents> for SigEventsFilter {
    fn filter(&self, event: &SigEvents) -> bool {
        self.0.contains(event.0)
    }
}

pub struct SigQueueObserver {
    wait_queue: WaitQueue,
    mask: SigMask,
    is_interrupted: AtomicBool,
}

impl SigQueueObserver {
    pub fn new(mask: SigMask) -> Arc<Self> {
        let wait_queue = WaitQueue::new();
        Arc::new(Self {
            wait_queue,
            mask,
            is_interrupted: AtomicBool::new(false),
        })
    }

    /// Wait until cond() returns Some(_).
    ///
    /// If some signal is caught before cond() returns Some(_), it will returns EINTR.
    pub fn wait_until_interruptible<F, R>(
        self: &Arc<Self>,
        mut cond: F,
        timeout: Option<&Duration>,
    ) -> Result<R>
    where
        F: FnMut() -> Option<R>,
    {
        self.is_interrupted.store(false, Ordering::Release);

        // Register observers on sigqueues

        let observer = Arc::downgrade(self) as Weak<dyn Observer<SigEvents>>;
        let filter = SigEventsFilter::new(self.mask);

        let current = current!();
        current.register_sigqueue_observer(observer.clone(), filter);

        let current_thread = current_thread!();
        let posix_thread = current_thread.as_posix_thread().unwrap();
        posix_thread.register_sigqueue_observer(observer.clone(), filter);

        // Some signal may come before we register observer, so we do another check here.
        if posix_thread.has_pending_signal() || current.has_pending_signal() {
            self.is_interrupted.store(true, Ordering::Release);
        }

        enum Res<R> {
            Ok(R),
            Interrupted,
        }

        let res = self.wait_queue.wait_until(
            || {
                if let Some(res) = cond() {
                    return Some(Res::Ok(res));
                }

                if self.is_interrupted.load(Ordering::Acquire) {
                    return Some(Res::Interrupted);
                }

                None
            },
            timeout,
        )?;

        current.unregiser_sigqueue_observer(&observer);
        posix_thread.unregiser_sigqueue_observer(&observer);

        match res {
            Res::Ok(r) => Ok(r),
            Res::Interrupted => return_errno_with_message!(Errno::EINTR, "interrupted by signal"),
        }
    }

    pub fn wait_until_uninterruptible<F, R>(&self, cond: F, timeout: Option<&Duration>) -> Result<R>
    where
        F: FnMut() -> Option<R>,
    {
        Ok(self.wait_queue.wait_until(cond, timeout)?)
    }
}

impl Observer<SigEvents> for SigQueueObserver {
    fn on_events(&self, events: &SigEvents) {
        self.is_interrupted.store(true, Ordering::Release);
        self.wait_queue.wake_all();
    }
}
