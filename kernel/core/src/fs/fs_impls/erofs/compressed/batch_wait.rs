// SPDX-License-Identifier: MPL-2.0

//! A bounded handoff from a completed batch to the public work queue.

use core::time::Duration;

use ostd::sync::WaitQueue;

use crate::{
    prelude::{Arc, Result, SpinLock},
    thread::work_queue::{self, WorkPriority, work_item::WorkItem},
    time::wait::{ManagedTimeout, WaitTimeout},
};

// Public enqueue does not wake idle workers. Keep one completed worker per
// filesystem briefly notifiable across measured sub-millisecond submission gaps;
// the single permit limits public-worker occupancy. Two milliseconds is an
// initial measured setting, not a hard bound on when the worker gets scheduled.
const BATCH_WAIT_TIMEOUT: Duration = Duration::from_millis(2);

/// Instance-local accounting that never owns jobs or worker threads.
pub(super) struct BatchWait {
    state: SpinLock<BatchWaitState>,
    posted: WaitQueue,
}

struct BatchWaitState {
    queued_batches: usize,
    waiting: bool,
    // Sticky until the current guard exits, even if another worker starts the job.
    notified: bool,
}

struct WaitGuard<'a> {
    owner: &'a BatchWait,
}

impl BatchWait {
    pub(super) fn new() -> Self {
        Self {
            state: SpinLock::new(BatchWaitState {
                queued_batches: 0,
                waiting: false,
                notified: false,
            }),
            posted: WaitQueue::new(),
        }
    }

    /// Publishes a newly allocated work item that has never been submitted.
    ///
    /// The caller must submit each item exactly once; reusable items are not supported.
    pub(super) fn submit_fresh(&self, item: Arc<WorkItem>) {
        let should_notify = {
            // Lock order: batch state -> public work queue. Enqueue only changes
            // atomics and an intrusive list; allocation and waking stay outside.
            let mut state = self.state.disable_irq().lock();
            state.queued_batches += 1;
            let accepted = work_queue::submit_work_item(item, WorkPriority::Normal);
            debug_assert!(accepted);
            if state.waiting {
                state.notified = true;
            }
            state.waiting
        };
        if should_notify {
            self.posted.wake_one();
        }
    }

    /// Removes the batch from queued accounting before processing any input.
    pub(super) fn on_start(&self) {
        let mut state = self.state.disable_irq().lock();
        debug_assert!(state.queued_batches > 0);
        state.queued_batches -= 1;
    }

    /// Briefly waits for another submission after all batch outputs are settled.
    pub(super) fn wait_after_completion(&self) {
        let Some(guard) = self.try_wait() else {
            return;
        };
        let _ = guard.wait();
    }

    fn try_wait(&self) -> Option<WaitGuard<'_>> {
        let mut state = self.state.disable_irq().lock();
        if state.queued_batches != 0 || state.waiting {
            return None;
        }
        state.waiting = true;
        state.notified = false;
        Some(WaitGuard { owner: self })
    }
}

impl WaitGuard<'_> {
    fn wait(&self) -> Result<()> {
        // Freeze once so spurious or delayed wakes cannot renew the interval.
        let mut deadline = ManagedTimeout::new(BATCH_WAIT_TIMEOUT);
        deadline.freeze();
        self.owner.posted.wait_until_or_timeout(
            || {
                let state = self.owner.state.disable_irq().lock();
                (state.notified || state.queued_batches != 0).then_some(())
            },
            deadline,
        )
    }
}

impl Drop for WaitGuard<'_> {
    fn drop(&mut self) {
        // Timed-out waiters leave closed wakers in the queue. Retain this permit
        // while draining them so a new waiter cannot register during cleanup.
        self.owner.posted.wake_all();
        let mut state = self.owner.state.disable_irq().lock();
        state.waiting = false;
        state.notified = false;
    }
}
