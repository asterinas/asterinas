// SPDX-License-Identifier: MPL-2.0

use core::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use crate::arch::timer::TIMER_FREQ;

/// Jiffies is a term used to denote the units of time measurement by the kernel.
///
/// A jiffy represents one tick of the system timer interrupt,
/// whose frequency is equal to [`TIMER_FREQ`] Hz.
#[derive(Copy, Clone, Debug)]
pub struct Jiffies(u64);

pub(crate) static ELAPSED: AtomicU64 = AtomicU64::new(0);

impl Jiffies {
    /// Creates a new instance.
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the elapsed time since the system boots up.
    pub fn elapsed() -> Self {
        Self::new(ELAPSED.load(Ordering::Relaxed))
    }

    /// Gets the number of jiffies.
    pub fn as_u64(self) -> u64 {
        self.0
    }

    /// Gets the [`Duration`] calculated from the jiffies counts.
    pub fn as_duration(self) -> Duration {
        Duration::from_millis(self.0 * 1000 / TIMER_FREQ)
    }
}

impl From<Jiffies> for Duration {
    fn from(value: Jiffies) -> Self {
        value.as_duration()
    }
}
