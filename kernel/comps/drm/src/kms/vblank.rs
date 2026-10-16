// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Weak;
use core::time::Duration;

use spin::Once;
use zerocopy::IntoBytes;

use crate::event::{DrmEvent, DrmEventBase, DrmEventReceiver, DrmEventType};

/// A vblank request whose completed userspace payload is immutable.
///
/// The CRTC owns the pending request; completion transfers it to the file's receiver.
/// The weak receiver reference lets the file disappear without cancelling each request.
#[derive(Debug)]
pub(crate) struct DrmVblankEvent {
    target_sequence: u64,
    user_data: u64,
    crtc_id: u32,
    receiver: Weak<DrmEventReceiver>,
    data: Once<DrmVblankEventData>,
}

impl DrmVblankEvent {
    pub(crate) fn new(
        target_sequence: u64,
        user_data: u64,
        crtc_id: u32,
        receiver: Weak<DrmEventReceiver>,
    ) -> Self {
        Self {
            target_sequence,
            user_data,
            crtc_id,
            receiver,
            data: Once::new(),
        }
    }

    /// Returns the userspace payload after completion.
    pub(crate) fn as_bytes(&self) -> &[u8] {
        self.data
            .get()
            .expect("only completed DRM events can be read")
            .as_bytes()
    }

    /// Completes and transfers this event to its receiver, if the receiver still exists.
    ///
    /// Uses the completion sequence and timestamp, which may precede the target
    /// when the CRTC shuts down.
    pub(crate) fn complete(self, sequence: u64, timestamp: Duration) {
        let Some(receiver) = self.receiver.upgrade() else {
            return;
        };

        self.data.call_once(|| {
            DrmVblankEventData::new(self.user_data, self.crtc_id, sequence, timestamp)
        });
        receiver.complete_event(DrmEvent::Vblank(self));
    }
}

/// `struct drm_event_vblank` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm.h#L1369-L1376>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct DrmVblankEventData {
    base: DrmEventBase,
    user_data: u64,
    tv_sec: u32,
    tv_usec: u32,
    sequence: u32,
    crtc_id: u32,
}

impl DrmVblankEventData {
    fn new(user_data: u64, crtc_id: u32, sequence: u64, timestamp: Duration) -> Self {
        let base = DrmEventBase::new(DrmEventType::Vblank, size_of::<Self>() as u32);

        Self {
            base,
            user_data,
            tv_sec: timestamp.as_secs() as u32,
            tv_usec: timestamp.subsec_micros(),
            // Preserves the legacy ABI's 32-bit modular sequence counter.
            sequence: sequence as u32,
            crtc_id,
        }
    }
}
