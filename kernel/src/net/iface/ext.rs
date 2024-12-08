// SPDX-License-Identifier: MPL-2.0

use alloc::string::String;
use core::sync::atomic::{AtomicU64, Ordering};

use aster_bigtcp::iface;
use ostd::sync::WaitQueue;

use crate::net::socket::ip::{datagram::DatagramObserver, stream::StreamObserver};

/// The iface extension.
pub struct IfaceExt {
    /// The name of the iface.
    name: String,
    /// The time when we should do the next poll.
    /// We store the total number of milliseconds since the system booted.
    next_poll_at_ms: AtomicU64,
    /// The wait queue that the background polling thread will sleep on.
    polling_wait_queue: WaitQueue,
}

impl IfaceExt {
    pub(super) fn new(name: String) -> Self {
        Self {
            name,
            next_poll_at_ms: AtomicU64::new(0),
            polling_wait_queue: WaitQueue::new(),
        }
    }

    /// Gets the name of the iface.
    ///
    /// In Linux, the name is usually the driver name followed by a unit number.
    pub(super) fn name(&self) -> &str {
        &self.name
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

impl iface::Ext for IfaceExt {
    type TcpEventObserver = StreamObserver;
    type UdpEventObserver = DatagramObserver;

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
