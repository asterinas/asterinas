// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

pub mod timer;

/// A trait that can abstract clocks which have the ability to read time,
/// and has a fixed resolution.
pub trait Clock: Send + Sync {
    /// Read the current time of this clock.
    fn read_time(&self) -> Duration;
}
