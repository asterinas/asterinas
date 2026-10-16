// SPDX-License-Identifier: MPL-2.0

//! KVM wall-clock RTC driver.
//!
//! The guest registers a page with KVM by writing its guest-physical address to
//! `MSR_KVM_WALL_CLOCK_NEW`, and KVM records a wall-clock epoch there.
//!
//! Reference: <https://elixir.bootlin.com/linux/v7.0/source/Documentation/virt/kvm/x86/msr.rst>

use core::time::Duration;

use chrono::DateTime;
use ostd::arch::kernel::KvmWallClock;

use super::Driver;
use crate::SystemTime;

pub(super) struct RtcKvmClock {
    clock: KvmWallClock,
}

impl Driver for RtcKvmClock {
    fn try_new() -> Option<Self> {
        let clock = KvmWallClock::setup()?;
        Some(Self { clock })
    }

    fn read_rtc(&self) -> SystemTime {
        match self
            .clock
            .read_wall_clock()
            .and_then(system_time_since_unix_epoch)
        {
            Some(system_time) => system_time,
            None => {
                ostd::warn!("Failed to obtain a valid KVM wall clock");
                unix_epoch()
            }
        }
    }
}

/// Converts a duration since the Unix epoch to a `SystemTime`.
fn system_time_since_unix_epoch(duration: Duration) -> Option<SystemTime> {
    let secs = i64::try_from(duration.as_secs()).ok()?;
    let datetime = DateTime::from_timestamp(secs, duration.subsec_nanos())?;
    Some(SystemTime::from(datetime.naive_utc()))
}

/// Returns the Unix epoch (1970-01-01 00:00:00).
fn unix_epoch() -> SystemTime {
    SystemTime {
        year: 1970,
        month: 1,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
        nanos: 0,
    }
}
