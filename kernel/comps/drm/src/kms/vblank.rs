// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::VecDeque, vec::Vec};
use core::{fmt::Debug, time::Duration};

use aster_core::{prelude::*, process::signal::Pause};
use ostd::sync::{Mutex, WaitQueue};

use crate::event::{DrmEventBase, DrmEventReservation, DrmEventResponse, DrmEventType};

// Preserves legacy 2^23-frame comparison window to distinguish
// reached targets from future targets across counter wraparound.
pub(crate) const MAX_PASSED_DISTANCE: u64 = 1 << 23;

/// A vblank request with reserved response capacity.
#[derive(Debug)]
pub(crate) struct DrmVblankRequest {
    target_sequence: u64,
    user_data: u64,
    crtc_id: u32,
    reservation: DrmEventReservation,
}

impl DrmVblankRequest {
    pub(crate) fn new(
        target_sequence: u64,
        user_data: u64,
        crtc_id: u32,
        reservation: DrmEventReservation,
    ) -> Self {
        Self {
            target_sequence,
            user_data,
            crtc_id,
            reservation,
        }
    }

    pub(crate) fn complete(self, sequence: u64, timestamp: Duration) -> Result<()> {
        let response = DrmVblankResponse::new(self.user_data, self.crtc_id, sequence, timestamp);
        self.reservation
            .complete_event(DrmEventResponse::Vblank(response))
    }
}

/// `struct drm_event_vblank` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.17/source/include/uapi/drm/drm.h#L1369-L1376>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct DrmVblankResponse {
    base: DrmEventBase,
    user_data: u64,
    tv_sec: u32,
    tv_usec: u32,
    sequence: u32,
    crtc_id: u32,
}

impl DrmVblankResponse {
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
            let events = core::mem::take(&mut state.requests);
            (state.sequence, state.timestamp, events)
        };

        self.wait_queue.wake_all();
        // Delivers the last snapshot after releasing the vblank state lock.
        for event in events {
            if let Err(error) = event.complete(sequence, timestamp) {
                ostd::error!("failed to complete DRM vblank request: {:?}", error);
            }
        }
    }

    /// Accepts an event while enabled, completing an already reached target immediately.
    ///
    /// Returns the target sequence for a queued event, or the current sequence
    /// for an immediate completion.
    pub(super) fn queue_request(&self, request: DrmVblankRequest) -> Result<u64> {
        let (sequence, timestamp) = {
            let mut state = self.state.lock();
            if !state.is_enabled {
                return_errno_with_message!(Errno::EINVAL, "the DRM vblank counter is disabled");
            }

            if !state.has_reached_vblank(request.target_sequence) {
                let target_sequence = request.target_sequence;
                state.requests.push_back(request);
                return Ok(target_sequence);
            }

            (state.sequence, state.timestamp)
        };

        // Completes an already reached target without waiting for another vblank.
        request.complete(sequence, timestamp)?;
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
            let pending_count = state.requests.len();

            for _ in 0..pending_count {
                let Some(event) = state.requests.pop_front() else {
                    break;
                };
                if !event.reservation.is_receiver_alive() {
                    continue;
                }
                if state.has_reached_vblank(event.target_sequence) {
                    completed_events.push(event);
                } else {
                    state.requests.push_back(event);
                }
            }

            (state.sequence, completed_events)
        };

        // Wakes synchronous ioctl callers to recheck their target sequences.
        self.wait_queue.wake_all();

        // Delivers completed events after releasing the vblank state lock.
        for event in completed_events {
            if let Err(error) = event.complete(sequence, timestamp) {
                ostd::error!("failed to complete DRM vblank request: {:?}", error);
            }
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
    requests: VecDeque<DrmVblankRequest>,
}

impl DrmVblankState {
    fn has_reached_vblank(&self, target_sequence: u64) -> bool {
        self.sequence.wrapping_sub(target_sequence) <= MAX_PASSED_DISTANCE
    }
}
