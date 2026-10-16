// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::VecDeque, sync::Weak, vec::Vec};
use core::{fmt::Debug, time::Duration};

use aster_core::{prelude::*, process::signal::Pause};
use ostd::sync::{Mutex, WaitQueue};
use spin::Once;
use zerocopy::IntoBytes;

use crate::event::{DrmEvent, DrmEventBase, DrmEventReceiver, DrmEventType};

// Preserves legacy 2^23-frame comparison window to distinguish
// reached targets from future targets across counter wraparound.
pub(crate) const MAX_PASSED_DISTANCE: u64 = 1 << 23;

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

/// The counter, pending events, and synchronous waiters for one CRTC.
///
/// The CRTC alone controls the running state through `set_enabled`, using its
/// enabled and active settings.
///
/// This type handles event delivery and waiting without acquiring the CRTC state lock
/// or calling back into the CRTC.
///
/// When both state locks are needed, the CRTC state lock comes first.
/// Waits release the vblank state lock before sleeping, so ticks and shutdown
/// can make progress.
///
/// Notifications and event delivery also run outside that lock.
pub(super) struct DrmVblank {
    state: Mutex<DrmVblankState>,
    wait_queue: WaitQueue,
}

impl DrmVblank {
    pub(super) fn sequence(&self) -> u64 {
        self.state.lock().sequence
    }

    /// Updates the CRTC-controlled running state and finishes outstanding requests on shutdown.
    ///
    /// Disabling wakes synchronous waiters and completes all pending events with
    /// the last sequence and timestamp, even if their targets have not been reached.
    /// Enabling preserves the counter and timestamp.
    pub(super) fn set_enabled(&self, enabled: bool) {
        let (sequence, timestamp, events) = {
            let mut state = self.state.lock();
            if state.is_enabled == enabled {
                return;
            }
            state.is_enabled = enabled;
            if enabled {
                return;
            }

            // Ends old waits even if the CRTC is re-enabled before they resume.
            state.stop_generation = state.stop_generation.wrapping_add(1);
            let events = core::mem::take(&mut state.events);
            (state.sequence, state.timestamp, events)
        };

        self.wait_queue.wake_all();
        // Delivers the last snapshot after releasing the vblank state lock.
        for event in events {
            event.complete(sequence, timestamp);
        }
    }

    /// Accepts an event while enabled, completing an already reached target immediately.
    ///
    /// Returns the target sequence for a queued event, or the current sequence
    /// for an immediate completion.
    ///
    /// The caller must reserve receiver capacity before calling this method
    /// and return that capacity if submission fails.
    pub(super) fn queue_event(&self, event: DrmVblankEvent) -> Result<u64> {
        let (sequence, timestamp) = {
            let mut state = self.state.lock();
            if !state.is_enabled {
                return_errno_with_message!(Errno::EINVAL, "the DRM vblank counter is disabled");
            }

            // Reclaims stale registrations even if the clock has not ticked recently.
            state
                .events
                .retain(|event| event.receiver.strong_count() != 0);

            if !state.has_reached_vblank(event.target_sequence) {
                let target_sequence = event.target_sequence;
                state.events.push_back(event);
                return Ok(target_sequence);
            }

            (state.sequence, state.timestamp)
        };

        // Completes an already reached target without waiting for another vblank.
        event.complete(sequence, timestamp);
        Ok(sequence)
    }

    /// Waits for the target, shutdown, a signal, or timeout.
    ///
    /// A shutdown during an accepted wait finishes it
    /// even if the CRTC is re-enabled before the waiter resumes.
    ///
    /// The caller must not hold the CRTC state lock while waiting, since shutdown
    /// needs that lock to disable vblank and wake the waiter.
    pub(super) fn wait_target_sequence(&self, target_sequence: u64) -> Result<DrmVblankWaitResult> {
        let stop_generation = {
            let state = self.state.lock();
            if !state.is_enabled {
                return_errno_with_message!(Errno::EINVAL, "the DRM vblank counter is disabled");
            }
            if state.has_reached_vblank(target_sequence) {
                return Ok(DrmVblankWaitResult::Finished {
                    sequence: state.sequence,
                    timestamp: state.timestamp,
                });
            }
            state.stop_generation
        };

        const WAIT_TIMEOUT: Duration = Duration::from_secs(3);
        let result = self.wait_queue.pause_until_or_timeout(
            || {
                let state = self.state.lock();
                (state.has_reached_vblank(target_sequence)
                    || !state.is_enabled
                    || state.stop_generation != stop_generation)
                    .then_some(DrmVblankWaitResult::Finished {
                        sequence: state.sequence,
                        timestamp: state.timestamp,
                    })
            },
            Some(&WAIT_TIMEOUT),
        );

        match result {
            Err(error) if error.error() == Errno::ETIME => {
                // Preserves a coherent snapshot for the ioctl's timeout reply.
                let state = self.state.lock();
                Ok(DrmVblankWaitResult::TimedOut {
                    sequence: state.sequence,
                    timestamp: state.timestamp,
                })
            }
            result => result,
        }
    }

    /// Advances the counter by `sequence_delta` and completes reached events.
    ///
    /// Uses the latest elapsed vblank's monotonic timestamp, including when
    /// several vblanks are reported together.
    ///
    /// Ignores zero increments and ticks while disabled.
    pub(super) fn handle_vblank(&self, timestamp: Duration, sequence_delta: u64) {
        if sequence_delta == 0 {
            return;
        }

        let (sequence, completed_events) = {
            let mut state = self.state.lock();
            if !state.is_enabled {
                return;
            }

            // Advances the sequence and its timestamp together, including missed vblanks.
            state.sequence = state.sequence.wrapping_add(sequence_delta);
            state.timestamp = timestamp;

            // Collects reached events while retaining requests for future vblanks.
            let mut completed_events = Vec::new();
            // Visits each pending event once, since future events are pushed back.
            let pending_count = state.events.len();

            for _ in 0..pending_count {
                let Some(event) = state.events.pop_front() else {
                    break;
                };
                if event.receiver.strong_count() == 0 {
                    continue;
                }
                if state.has_reached_vblank(event.target_sequence) {
                    completed_events.push(event);
                } else {
                    state.events.push_back(event);
                }
            }

            (state.sequence, completed_events)
        };

        // Wakes synchronous ioctl callers to recheck their target sequences.
        self.wait_queue.wake_all();

        // Delivers completed events after releasing the vblank state lock.
        for event in completed_events {
            event.complete(sequence, timestamp);
        }
    }
}

impl Debug for DrmVblank {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DrmVblank")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl Default for DrmVblank {
    fn default() -> Self {
        Self {
            state: Mutex::new(DrmVblankState::default()),
            wait_queue: WaitQueue::new(),
        }
    }
}

/// The sequence and timestamp observed when a synchronous vblank wait ends.
#[derive(Debug)]
pub(crate) enum DrmVblankWaitResult {
    /// A reached target or a wait ended by CRTC shutdown.
    Finished { sequence: u64, timestamp: Duration },
    /// A snapshot taken after the wait timed out.
    TimedOut { sequence: u64, timestamp: Duration },
}

/// The vblank snapshot, running state, and pending requests protected by one lock.
#[derive(Debug, Default)]
struct DrmVblankState {
    /// The running state set only by CRTC state transitions.
    is_enabled: bool,
    /// The shutdown generation used to end waits across a disable/enable cycle.
    stop_generation: u64,
    /// The wrapping vblank counter, preserved across disable/enable cycles.
    sequence: u64,
    /// The monotonic timestamp of `sequence`.
    timestamp: Duration,
    /// The owned pending requests, pruned when their receivers disappear.
    events: VecDeque<DrmVblankEvent>,
}

impl DrmVblankState {
    fn has_reached_vblank(&self, target_sequence: u64) -> bool {
        self.sequence.wrapping_sub(target_sequence) <= MAX_PASSED_DISTANCE
    }
}
