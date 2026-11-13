// SPDX-License-Identifier: MPL-2.0

use alloc::sync::{Arc, Weak};
use core::{fmt::Debug, num::NonZeroU32, time::Duration};

use aster_core::{prelude::*, thread::kernel_thread::ThreadOptions, time::wait::WaitTimeout};
use ostd::sync::{Mutex, WaitQueue};

use crate::kms::vblank::DrmVblank;

#[derive(Debug)]
pub struct DrmSoftwareVblank {
    inner: Arc<DrmSoftwareVblankInner>,
}

impl DrmSoftwareVblank {
    pub fn new(vblank: &Arc<DrmVblank>, refresh_hz: NonZeroU32) -> Self {
        let software = Self {
            inner: Arc::new(DrmSoftwareVblankInner {
                state: Mutex::new(SoftwareVblankState::Stopped),
                vblank: Arc::downgrade(vblank),
                refresh_hz,
                wait_queue: WaitQueue::new(),
            }),
        };

        // Keep only the inner alive so dropping the service can request shutdown.
        let inner = software.inner.clone();
        ThreadOptions::new(move || Self::run(inner)).build().run();

        software
    }

    fn run(inner: Arc<DrmSoftwareVblankInner>) {
        loop {
            // Remain idle until enabled.
            inner.wait_queue.wait_until(|| {
                (!matches!(&*inner.state.lock(), SoftwareVblankState::Stopped)).then_some(())
            });

            let (next_deadline, elapsed_vblank) = {
                let mut state = inner.state.lock();
                match &mut *state {
                    SoftwareVblankState::Stopped => continue,
                    SoftwareVblankState::Shutdown => return,
                    SoftwareVblankState::Running(timeline) => {
                        let elapsed_vblank =
                            timeline.advance_until_now(aster_time::read_monotonic_time());
                        (timeline.next_deadline(), elapsed_vblank)
                    }
                }
            };

            if let Some((timestamp, sequence_delta)) = elapsed_vblank {
                let Some(vblank) = inner.vblank.upgrade() else {
                    return;
                };
                vblank.handle_vblank(timestamp, sequence_delta);
                continue;
            }

            let timeout = next_deadline.saturating_sub(aster_time::read_monotonic_time());
            let result = inner.wait_queue.wait_until_or_timeout(
                || {
                    let state = inner.state.lock();
                    match &*state {
                        SoftwareVblankState::Running(timeline) => {
                            // Recompute the timeout if a restart replaced the timeline.
                            (timeline.next_deadline() != next_deadline).then_some(())
                        }
                        _ => Some(()),
                    }
                },
                &timeout,
            );

            match result {
                Ok(()) => {}
                Err(error) if error.error() == Errno::ETIME => {}
                Err(_) => return,
            }
        }
    }

    pub fn enable(&self) -> Result<()> {
        {
            let mut state = self.inner.state.lock();
            match &*state {
                SoftwareVblankState::Stopped => {
                    *state = SoftwareVblankState::Running(VblankTimeline::new(
                        aster_time::read_monotonic_time(),
                        self.inner.refresh_hz,
                    ));
                }
                SoftwareVblankState::Running(_) => return Ok(()),
                SoftwareVblankState::Shutdown => return Err(Errno::ENODEV.into()),
            }
        }
        self.inner.wait_queue.wake_all();
        Ok(())
    }

    pub fn disable(&self) {
        {
            let mut state = self.inner.state.lock();
            if matches!(&*state, SoftwareVblankState::Running(_)) {
                *state = SoftwareVblankState::Stopped;
            }
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

struct DrmSoftwareVblankInner {
    state: Mutex<SoftwareVblankState>,
    vblank: Weak<DrmVblank>,
    refresh_hz: NonZeroU32,
    wait_queue: WaitQueue,
}

impl Debug for DrmSoftwareVblankInner {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DrmSoftwareVblankInner")
            .field("refresh_hz", &self.refresh_hz)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
enum SoftwareVblankState {
    Stopped,
    Running(VblankTimeline),
    Shutdown,
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
    fn advance_until_now(&mut self, now: Duration) -> Option<(Duration, u64)> {
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

        Some((timestamp, sequence_delta as u64))
    }
}
