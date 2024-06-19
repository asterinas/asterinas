// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use aster_time::NANOS_PER_SECOND;
use ostd::arch::timer::TIMER_FREQ;

pub mod timer;

type Nanos = u64;

/// A trait that can abstract clocks which have the ability to read time,
/// and has a fixed resolution.
pub trait Clock: Send + Sync {
    /// Read the current time of this clock.
    fn read_time(&self) -> Duration;

    /// Return the resolution of this clock.
    /// Set to the resolution of system time interrupt by default.
    fn resolution(&self) -> Nanos
    where
        Self: Sized,
    {
        NANOS_PER_SECOND as u64 / TIMER_FREQ
    }
}
