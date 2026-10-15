// SPDX-License-Identifier: MPL-2.0

//! A software vblank clock for simpledrm's fixed refresh rate.

use alloc::sync::Arc;
use core::{fmt::Debug, time::Duration};

use aster_core::{thread::kernel_thread::ThreadOptions, time::wait::WaitTimeout};
use aster_drm::kms::objects::crtc::DrmCrtc;
use ostd::sync::{Mutex, WaitQueue};

/// A task-context clock that runs while the simpledrm CRTC is enabled.
#[derive(Debug)]
pub(super) struct SimpleDrmVblankClock {
    inner: Arc<ClockInner>,
}

impl SimpleDrmVblankClock {
    pub(super) fn new(crtc: Arc<DrmCrtc>) -> Self {
        let inner = Arc::new(ClockInner {
            crtc,
            state: Mutex::new(ClockState::Disabled),
            wait_queue: WaitQueue::new(),
        });
        let thread_inner = inner.clone();
        ThreadOptions::new(move || thread_inner.run()).build().run();

        Self { inner }
    }

    /// Starts the clock without resetting an already active frame schedule.
    pub(super) fn start(&self) {
        {
            let mut state = self.inner.state.lock();
            if !matches!(*state, ClockState::Disabled) {
                return;
            }
            *state = ClockState::Running(FrameSchedule::new(aster_time::read_monotonic_time()));
        }
        self.inner.wait_queue.wake_all();
    }

    /// Stops the clock and waits for any in-progress vblank handling to finish.
    pub(super) fn stop(&self) {
        {
            let mut state = self.inner.state.lock();
            if !matches!(*state, ClockState::Running(_)) {
                return;
            }
            *state = ClockState::Disabled;
        }
        self.inner.wait_queue.wake_all();
    }
}

impl Drop for SimpleDrmVblankClock {
    fn drop(&mut self) {
        *self.inner.state.lock() = ClockState::Shutdown;
        self.inner.wait_queue.wake_all();
    }
}

struct ClockInner {
    crtc: Arc<DrmCrtc>,
    // Serializes start/stop with vblank delivery; the thread never takes the object-store lock.
    state: Mutex<ClockState>,
    wait_queue: WaitQueue,
}

impl Debug for ClockInner {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ClockInner")
            .field("crtc", &self.crtc)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl ClockInner {
    fn run(&self) {
        loop {
            let next_deadline = {
                let mut state = self.state.lock();
                match &mut *state {
                    ClockState::Shutdown => return,
                    ClockState::Disabled => None,
                    ClockState::Running(schedule) => {
                        let now = aster_time::read_monotonic_time();
                        if let Some((elapsed_frames, timestamp)) = schedule.take_due_timestamp(now)
                        {
                            self.crtc.handle_vblank(timestamp, elapsed_frames);
                        }
                        Some(schedule.next_deadline())
                    }
                }
            };

            self.wait_for_next_frame(next_deadline);
        }
    }

    /// Waits for a frame deadline or a change to the clock state.
    fn wait_for_next_frame(&self, next_deadline: Option<Duration>) {
        let timeout = next_deadline
            .map(|deadline| deadline.saturating_sub(aster_time::read_monotonic_time()));
        let _ = self.wait_queue.wait_until_or_timeout(
            || {
                let state = self.state.lock();
                (matches!(*state, ClockState::Shutdown) || state.next_deadline() != next_deadline)
                    .then_some(())
            },
            timeout.as_ref(),
        );
        // Removes closed wakers left by timed-out waits; only the clock thread waits here.
        self.wait_queue.wake_all();
    }
}

#[derive(Debug)]
enum ClockState {
    Disabled,
    Running(FrameSchedule),
    Shutdown,
}

impl ClockState {
    fn next_deadline(&self) -> Option<Duration> {
        match self {
            Self::Running(schedule) => Some(schedule.next_deadline()),
            Self::Disabled | Self::Shutdown => None,
        }
    }
}

#[derive(Debug)]
struct FrameSchedule {
    epoch: Duration,
    next_frame: u64,
}

impl FrameSchedule {
    fn new(epoch: Duration) -> Self {
        Self {
            epoch,
            next_frame: 1,
        }
    }

    fn next_deadline(&self) -> Duration {
        self.deadline_for_frame(self.next_frame)
    }

    fn deadline_for_frame(&self, frame: u64) -> Duration {
        let refresh_hz = u64::from(crate::SIMPLEDRM_VREFRESH_HZ);
        // Derives deadlines from the epoch to avoid accumulating rounding and wakeup delays.
        let elapsed = Duration::new(
            frame / refresh_hz,
            ((frame % refresh_hz) * 1_000_000_000 / refresh_hz) as u32,
        );
        self.epoch + elapsed
    }

    /// Skips expired deadlines and returns their count with the latest frame timestamp.
    fn take_due_timestamp(&mut self, now: Duration) -> Option<(u64, Duration)> {
        if self.next_deadline() > now {
            return None;
        }

        let elapsed = now - self.epoch;
        let refresh_hz = u64::from(crate::SIMPLEDRM_VREFRESH_HZ);
        // Includes frame boundaries rounded down to the preceding nanosecond.
        let last_frame = elapsed.as_secs() * refresh_hz
            + ((u64::from(elapsed.subsec_nanos()) + 1) * refresh_hz - 1) / 1_000_000_000;
        let elapsed_frames = last_frame - self.next_frame + 1;
        let timestamp = self.deadline_for_frame(last_frame);

        // Sets the next frame to the first one that has not occurred yet.
        self.next_frame = last_frame + 1;
        Some((elapsed_frames, timestamp))
    }
}
