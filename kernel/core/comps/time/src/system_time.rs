// SPDX-License-Identifier: MPL-2.0

use core::{ops::Add, time::Duration};

/// A system time, recorded as the duration since the Unix epoch.
#[derive(Clone, Copy)]
pub struct SystemTime(Duration);

impl SystemTime {
    /// The Unix epoch (1970-01-01 00:00:00).
    pub const UNIX_EPOCH: Self = Self(Duration::ZERO);

    /// Creates a new system time from the duration since the Unix epoch.
    pub fn new(duration: Duration) -> Self {
        Self(duration)
    }

    /// Returns the duration since the Unix epoch.
    pub fn since_epoch(self) -> Duration {
        self.0
    }
}

impl Add<Duration> for SystemTime {
    type Output = SystemTime;

    fn add(self, rhs: Duration) -> Self::Output {
        // We chose to use `saturating_add` to prevent panics if a buggy RTC reports an excessive
        // duration near `Duration::MAX`.
        //
        // This means we'll lose some arithmetic properties, but as long as we only implement
        // `Add`, it should be fine. For now, we don't implement other operations like `Sub`.
        Self(self.0.saturating_add(rhs))
    }
}
