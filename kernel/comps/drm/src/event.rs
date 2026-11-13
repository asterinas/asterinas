// SPDX-License-Identifier: MPL-2.0

use alloc::{
    collections::VecDeque,
    sync::{Arc, Weak},
};
use core::fmt::Debug;

use aster_core::{
    events::IoEvents,
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
};
use ostd::{
    mm::{FallibleVmWrite, VmReader, VmWriter},
    sync::Mutex,
};
use zerocopy::IntoBytes;

use crate::kms::vblank::DrmVblankResponse;

/// A per-file event receiver.
pub(crate) struct DrmEventReceiver {
    pollee: Pollee,
    queue: Mutex<DrmEventQueue>,
}

impl DrmEventReceiver {
    /// Copies whole events in completion order without waiting for new events.
    ///
    /// Returns `EAGAIN` if no events are ready.
    ///
    /// A buffer too small for the next event leaves it queued
    /// and may result in a zero-byte read.
    pub(super) fn try_read(&self, writer: &mut VmWriter) -> Result<usize> {
        let mut queue = self.queue.lock();
        let mut total_written = 0;

        loop {
            let Some(event) = queue.ready_events.front() else {
                if total_written != 0 {
                    return Ok(total_written);
                }
                return_errno_with_message!(Errno::EAGAIN, "the DRM file has no events");
            };

            if writer.avail() < event.len() {
                return Ok(total_written);
            }
            let mut reader = VmReader::from(event.as_bytes());
            if let Err(err) = writer.write_fallible(&mut reader) {
                // Reports completed events and leaves the failed event queued for retry.
                if total_written != 0 {
                    return Ok(total_written);
                }
                return Err(err.into());
            }

            let len = event.len();
            total_written += len;
            queue.ready_events.pop_front();
            queue.available_bytes += len;
            self.pollee.invalidate();
        }
    }

    /// Reserves capacity shared by pending and unread events.
    pub(crate) fn reserve_event_space(
        self: &Arc<Self>,
        length: usize,
    ) -> Result<DrmEventReservation> {
        let mut queue = self.queue.lock();
        queue.available_bytes = queue
            .available_bytes
            .checked_sub(length)
            .ok_or_else(|| Error::with_message(Errno::ENOMEM, "the DRM event buffer is full"))?;

        Ok(DrmEventReservation {
            receiver: Arc::downgrade(self),
            reserved_bytes: length,
        })
    }
}

impl Debug for DrmEventReceiver {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DrmEventReceiver")
            .field("queue", &self.queue)
            .finish_non_exhaustive()
    }
}

impl Default for DrmEventReceiver {
    fn default() -> Self {
        Self {
            pollee: Pollee::new(),
            queue: Mutex::new(DrmEventQueue::default()),
        }
    }
}

impl Pollable for DrmEventReceiver {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee.poll_with(mask, poller, || {
            if self.queue.lock().ready_events.is_empty() {
                IoEvents::empty()
            } else {
                IoEvents::IN | IoEvents::RDNORM
            }
        })
    }
}

/// A byte-bounded queue of completed DRM events.
#[derive(Debug)]
struct DrmEventQueue {
    ready_events: VecDeque<DrmEventResponse>,
    /// The unreserved bytes, excluding both pending and unread events.
    available_bytes: usize,
}

impl DrmEventQueue {
    const DRM_EVENT_QUEUE_CAPACITY_BYTES: usize = 4096;
}

impl Default for DrmEventQueue {
    fn default() -> Self {
        Self {
            ready_events: VecDeque::new(),
            available_bytes: Self::DRM_EVENT_QUEUE_CAPACITY_BYTES,
        }
    }
}

/// Reserved event capacity.
#[must_use]
#[derive(Debug)]
pub(crate) struct DrmEventReservation {
    receiver: Weak<DrmEventReceiver>,
    reserved_bytes: usize,
}

impl DrmEventReservation {
    pub(crate) fn consume_with_response(mut self, response: DrmEventResponse) {
        let Some(receiver) = self.receiver.upgrade() else {
            return;
        };

        {
            let mut queue = receiver.queue.lock();
            queue.ready_events.push_back(response);
            // Keeps the capacity reserved until the queued response is read.
            self.reserved_bytes = 0;
        }
        receiver.pollee.notify(IoEvents::IN | IoEvents::RDNORM);
    }
}

impl Drop for DrmEventReservation {
    fn drop(&mut self) {
        if self.reserved_bytes == 0 {
            return;
        }

        if let Some(receiver) = self.receiver.upgrade() {
            receiver.queue.lock().available_bytes += self.reserved_bytes;
        }
    }
}

/// A completed event readable through the DRM file.
#[derive(Debug)]
pub(crate) enum DrmEventResponse {
    Vblank(DrmVblankResponse),
}

impl DrmEventResponse {
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Vblank(response) => size_of_val(response),
        }
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Vblank(response) => response.as_bytes(),
        }
    }
}

/// `struct drm_event` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm.h#L1323-L1340>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct DrmEventBase {
    type_: u32,
    length: u32,
}

impl DrmEventBase {
    pub(crate) fn new(type_: DrmEventType, length: u32) -> Self {
        Self {
            type_: type_ as u32,
            length,
        }
    }
}

/// The event type identifiers in the DRM userspace ABI.
#[repr(u32)]
#[derive(Debug)]
pub(crate) enum DrmEventType {
    Vblank = 0x01,
}
