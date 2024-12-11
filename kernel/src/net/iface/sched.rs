// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU64, Ordering};

use aster_bigtcp::iface::ScheduleNextPoll;
use ostd::sync::WaitQueue;

pub struct PollScheduler {
    /// The time when we should do the next poll.
    /// We store the total number of milliseconds since the system booted.
    next_poll_at_ms: AtomicU64,
    /// The wait queue that the background polling thread will sleep on.
    polling_wait_queue: WaitQueue,
}

impl PollScheduler {
    pub(super) fn new() -> Self {
        Self {
            next_poll_at_ms: AtomicU64::new(0),
            polling_wait_queue: WaitQueue::new(),
        }
    }

    pub(super) fn next_poll_at_ms(&self) -> Option<u64> {
        let millis = self.next_poll_at_ms.load(Ordering::Relaxed);
        if millis == 0 {
            None
        } else {
            Some(millis)
        }
    }

    pub(super) fn polling_wait_queue(&self) -> &WaitQueue {
        &self.polling_wait_queue
    }
}

impl ScheduleNextPoll for PollScheduler {
    fn schedule_next_poll(&self, poll_at: Option<u64>) {
        let Some(new_instant) = poll_at else {
            self.next_poll_at_ms.store(0, Ordering::Relaxed);
            return;
        };

        let old_instant = self.next_poll_at_ms.load(Ordering::Relaxed);
        self.next_poll_at_ms.store(new_instant, Ordering::Relaxed);

        if old_instant == 0 || new_instant < old_instant {
            self.polling_wait_queue.wake_all();
        }
    }
}
