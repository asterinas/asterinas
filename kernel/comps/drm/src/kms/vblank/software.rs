// SPDX-License-Identifier: MPL-2.0

//! A software event source for a CRTC's vblank tracking.

use alloc::sync::{Arc, Weak};
use core::{fmt::Debug, num::NonZeroU32, time::Duration};

use aster_core::{thread::kernel_thread::ThreadOptions, time::wait::WaitTimeout};
use ostd::sync::{Mutex, WaitQueue};

use crate::kms::objects::crtc::DrmCrtc;

/// A fixed-refresh vblank source owned and controlled by its CRTC.
#[derive(Debug)]
pub struct DrmSoftwareVblank {
    inner: Arc<SoftwareVblankInner>,
}

impl DrmSoftwareVblank {
    pub fn new(crtc: Weak<DrmCrtc>, refresh_hz: NonZeroU32) -> Self {
        Self {
            inner: Arc::new(SoftwareVblankInner {
                crtc,
                refresh_hz,
                state: Mutex::new(SoftwareVblankState::NotStarted),
                wait_queue: WaitQueue::new(),
            }),
        }
    }

    /// Starts the source without resetting an already running timeline.
    ///
    /// Creates the worker on the first call and reuses it on subsequent starts.
    pub(crate) fn start(&self) {
        let should_spawn = {
            let mut state = self.inner.state.lock();
            let should_spawn = match &*state {
                SoftwareVblankState::NotStarted => true,
                SoftwareVblankState::Disabled => false,
                SoftwareVblankState::Running(_) | SoftwareVblankState::Shutdown => return,
            };

            *state = SoftwareVblankState::Running(VblankTimeline::new(
                aster_time::read_monotonic_time(),
                self.inner.refresh_hz,
            ));
            should_spawn
        };

        if should_spawn {
            let inner = self.inner.clone();
            ThreadOptions::new(move || inner.run()).build().run();
        } else {
            self.inner.wait_queue.wake_all();
        }
    }

    /// Stops the source and waits for any in-progress vblank handling to finish.
    ///
    /// The caller must not hold the vblank state lock.
    pub(crate) fn stop(&self) {
        {
            let mut state = self.inner.state.lock();
            if !matches!(*state, SoftwareVblankState::Running(_)) {
                return;
            }
            *state = SoftwareVblankState::Disabled;
        }
        self.inner.wait_queue.wake_all();
    }
}

impl Drop for DrmSoftwareVblank {
    fn drop(&mut self) {
        *self.inner.state.lock() = SoftwareVblankState::Shutdown;
        self.inner.wait_queue.wake_all();
    }
}

struct SoftwareVblankInner {
    crtc: Weak<DrmCrtc>,
    refresh_hz: NonZeroU32,
    state: Mutex<SoftwareVblankState>,
    wait_queue: WaitQueue,
}

impl Debug for SoftwareVblankInner {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SoftwareVblankInner")
            .field("refresh_hz", &self.refresh_hz)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl SoftwareVblankInner {
    fn run(&self) {
        loop {
            let Some(crtc) = self.crtc.upgrade() else {
                return;
            };

            let next_deadline = {
                let mut state = self.state.lock();
                match &mut *state {
                    SoftwareVblankState::Shutdown => return,
                    SoftwareVblankState::NotStarted | SoftwareVblankState::Disabled => None,
                    SoftwareVblankState::Running(timeline) => {
                        let now = aster_time::read_monotonic_time();
                        if let Some((sequence_delta, timestamp)) = timeline.take_due_timestamp(now)
                        {
                            crtc.vblank().handle_vblank(timestamp, sequence_delta);
                        }
                        Some(timeline.next_deadline())
                    }
                }
            };

            // Releasing the last CRTC reference can shut down this source.
            // Drop it outside the source lock, and never keep the CRTC alive across sleep.
            drop(crtc);
            self.wait_for_next_frame(next_deadline);
        }
    }

    fn wait_for_next_frame(&self, next_deadline: Option<Duration>) {
        let timeout = next_deadline
            .map(|deadline| deadline.saturating_sub(aster_time::read_monotonic_time()));
        let _ = self.wait_queue.wait_until_or_timeout(
            || {
                let state = self.state.lock();
                (matches!(*state, SoftwareVblankState::Shutdown)
                    || state.next_deadline() != next_deadline)
                    .then_some(())
            },
            timeout.as_ref(),
        );
    }
}

#[derive(Debug)]
enum SoftwareVblankState {
    NotStarted,
    Disabled,
    Running(VblankTimeline),
    Shutdown,
}

impl SoftwareVblankState {
    fn next_deadline(&self) -> Option<Duration> {
        match self {
            Self::Running(timeline) => Some(timeline.next_deadline()),
            Self::NotStarted | Self::Disabled | Self::Shutdown => None,
        }
    }
}

#[derive(Debug)]
struct VblankTimeline {
    epoch: Duration,
    next_frame: u128,
    refresh_hz: NonZeroU32,
}

impl VblankTimeline {
    fn new(epoch: Duration, refresh_hz: NonZeroU32) -> Self {
        Self {
            epoch,
            next_frame: 1,
            refresh_hz,
        }
    }

    fn next_deadline(&self) -> Duration {
        self.deadline_for_frame(self.next_frame)
    }

    fn deadline_for_frame(&self, frame: u128) -> Duration {
        let refresh_hz = u128::from(self.refresh_hz.get());
        let elapsed = Duration::new(
            (frame / refresh_hz) as u64,
            ((frame % refresh_hz) * 1_000_000_000 / refresh_hz) as u32,
        );
        self.epoch.saturating_add(elapsed)
    }

    /// Returns elapsed frames with the timestamp of their last frame boundary.
    fn take_due_timestamp(&mut self, now: Duration) -> Option<(u64, Duration)> {
        if self.next_deadline() > now {
            return None;
        }

        // Derives deadlines from the epoch to avoid accumulating rounding and wakeup delays.
        let elapsed = now.saturating_sub(self.epoch);
        let refresh_hz = u128::from(self.refresh_hz.get());
        // Includes frame boundaries rounded down to the preceding nanosecond.
        let last_frame = ((elapsed.as_nanos() + 1) * refresh_hz - 1) / 1_000_000_000;
        let sequence_delta = last_frame - self.next_frame + 1;
        let timestamp = self.deadline_for_frame(last_frame);
        self.next_frame = last_frame + 1;

        Some((sequence_delta as u64, timestamp))
    }
}
