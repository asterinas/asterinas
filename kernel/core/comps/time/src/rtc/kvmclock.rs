// SPDX-License-Identifier: MPL-2.0

//! KVM wall-clock RTC driver.
//!
//! KVM records the wall-clock time at the moment of guest boot
//! in a page shared with the guest.
//! The guest passes the page to KVM by writing its guest-physical address
//! to `MSR_KVM_WALL_CLOCK_NEW`,
//! and KVM fills the page at the moment of that write.
//! This driver reads the page to provide the wall-clock time at guest boot,
//! which the kernel uses to initialize the system clock.
//!
//! Reference:
//! <https://elixir.bootlin.com/linux/v7.0/source/Documentation/virt/kvm/x86/msr.rst>

use chrono::DateTime;
use ostd::arch::kernel::{self, KvmWallClock};

use super::Driver;
use crate::SystemTime;

pub(super) struct RtcKvmClock;

impl Driver for RtcKvmClock {
    fn try_new() -> Option<Self> {
        kernel::has_kvm_clocksource2().then_some(Self)
    }

    fn read_rtc(&self) -> SystemTime {
        match kernel::read_kvm_wall_clock().and_then(wall_clock_to_system_time) {
            Some(system_time) => system_time,
            None => {
                ostd::warn!("Failed to obtain a valid KVM wall clock");
                unix_epoch()
            }
        }
    }
}

fn wall_clock_to_system_time(wall_clock: KvmWallClock) -> Option<SystemTime> {
    let datetime = DateTime::from_timestamp(wall_clock.sec() as i64, wall_clock.nsec())?;
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
