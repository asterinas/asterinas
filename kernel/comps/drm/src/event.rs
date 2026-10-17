// SPDX-License-Identifier: MPL-2.0

use alloc::collections::VecDeque;
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

use crate::kms::vblank::{DrmVblankEvent, DrmVblankEventData};

const DRM_EVENT_CAPACITY_BYTES: usize = 4096;

/// A per-file queue of completed events waiting to be read.
///
/// Pending and unread events share one capacity budget.
/// Completion keeps the reservation; reading a whole event releases it.
///
/// Pending events hold weak references and do not keep this receiver alive.
/// Producers discard them when they find that the receiver is gone.
pub(crate) struct DrmEventReceiver {
    pollee: Pollee,
    state: Mutex<DrmEventReceiverState>,
}

impl DrmEventReceiver {
    /// Copies whole events in completion order without waiting for new events.
    ///
    /// Returns `EAGAIN` if no events are ready.
    ///
    /// A buffer too small for the next event leaves it queued
    /// and may result in a zero-byte read.
    pub(super) fn try_read_events(&self, writer: &mut VmWriter) -> Result<usize> {
        let mut state = self.state.lock();
        let mut total_written = 0;

        loop {
            let Some(event) = state.ready_event.front() else {
                if total_written != 0 {
                    return Ok(total_written);
                }
                return_errno_with_message!(Errno::EAGAIN, "the DRM file has no events");
            };
            // Leaves the event queued if it does not fit in the remaining buffer.
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
            state.ready_event.pop_front();
            state.available_bytes += len;
            self.pollee.invalidate();
        }
    }

    /// Reserves capacity shared by pending and unread events.
    ///
    /// Requires reservation before submission, since submission may complete
    /// immediately.
    ///
    /// Failed submission must release the reservation explicitly.
    pub(crate) fn reserve_event_space(&self, length: usize) -> Result<()> {
        let mut state = self.state.lock();
        state.available_bytes = state
            .available_bytes
            .checked_sub(length)
            .ok_or_else(|| Error::with_message(Errno::ENOMEM, "the DRM event buffer is full"))?;

        Ok(())
    }

    /// Returns capacity reserved for an event that could not be queued.
    pub(crate) fn release_event_space(&self, length: usize) {
        self.state.lock().available_bytes += length;
    }

    /// Queues a completed event and notifies readers, retaining its reserved capacity.
    ///
    /// Requires an initialized payload and capacity already reserved for the event.
    pub(crate) fn complete_event(&self, event: DrmEvent) {
        self.state.lock().ready_event.push_back(event);
        self.pollee.notify(IoEvents::IN);
    }
}

impl Debug for DrmEventReceiver {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DrmEventReceiver")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl Default for DrmEventReceiver {
    fn default() -> Self {
        Self {
            pollee: Pollee::new(),
            state: Mutex::new(DrmEventReceiverState {
                ready_event: VecDeque::new(),
                available_bytes: DRM_EVENT_CAPACITY_BYTES,
            }),
        }
    }
}

impl Pollable for DrmEventReceiver {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee.poll_with(mask, poller, || {
            if self.state.lock().ready_event.is_empty() {
                IoEvents::empty()
            } else {
                IoEvents::IN
            }
        })
    }
}

/// The completed event queue and remaining capacity for this file.
#[derive(Debug)]
struct DrmEventReceiverState {
    ready_event: VecDeque<DrmEvent>,
    /// The unreserved bytes, excluding both pending and unread events.
    available_bytes: usize,
}

/// A completed event owned by the receiver and readable through the DRM file.
#[derive(Debug)]
pub(crate) enum DrmEvent {
    Vblank(DrmVblankEvent),
}

impl DrmEvent {
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Vblank(_) => size_of::<DrmVblankEventData>(),
        }
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Vblank(event) => event.as_bytes(),
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
