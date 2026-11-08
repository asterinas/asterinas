// SPDX-License-Identifier: MPL-2.0

//! KVM wall-clock RTC driver.
//!
//! The guest registers pages with KVM by writing their guest-physical
//! addresses to `MSR_KVM_SYSTEM_TIME_NEW` and `MSR_KVM_WALL_CLOCK_NEW`. KVM
//! writes the pvclock data and a wall-clock epoch into these pages.
//!
//! Reference: <https://elixir.bootlin.com/linux/v7.0/source/Documentation/virt/kvm/x86/msr.rst>

use core::time::Duration;

use ostd::arch::{
    kernel::{PvclockTimeSnapshot, read_pvclock_time_snapshot, read_wall_clock_epoch},
    read_tsc,
};

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
        let duration = self.clock.read_wall_clock();
        SystemTime::new(duration)
    }
}

/// The KVM wall clock.
///
/// The current time is the wall-clock epoch plus the pvclock value
/// extrapolated to the current TSC.
struct KvmWallClock {
    epoch: Duration,
    snapshot: PvclockTimeSnapshot,
}

impl KvmWallClock {
    /// Prepares the KVM wall clock.
    fn setup() -> Option<Self> {
        let epoch = read_wall_clock_epoch()?;
        let snapshot = read_pvclock_time_snapshot()?;

        if (snapshot.tsc_shift as i32).unsigned_abs() >= 64 {
            return None;
        }

        Some(Self { epoch, snapshot })
    }

    /// Returns the current wall-clock time as the duration since the Unix
    /// epoch.
    fn read_wall_clock(&self) -> Duration {
        let clock_ns = snapshot_clock_ns(&self.snapshot, read_tsc());

        // This won't fail because the duration is the epoch (bounded by
        // 2^32 seconds) plus the guest uptime (bounded by 2^64 nanoseconds).
        self.epoch + Duration::from_nanos(clock_ns)
    }
}

/// Returns the KVM clock value in nanoseconds extrapolated to `tsc`.
fn snapshot_clock_ns(snapshot: &PvclockTimeSnapshot, tsc: u64) -> u64 {
    // TSC wraps, so use wrapping subtraction.
    let delta = tsc.wrapping_sub(snapshot.tsc_timestamp);
    snapshot
        .system_time
        .saturating_add(scale_delta(snapshot, delta))
}

/// Converts a TSC delta to nanoseconds.
fn scale_delta(snapshot: &PvclockTimeSnapshot, delta: u64) -> u64 {
    // This computes `((delta << tsc_shift) * tsc_to_system_mul) >> 32`
    // without losing precision.

    // `val` has a maximum of 96 bits, 64 of which come from `delta` and
    // 32 from `tsc_to_system_mul`.
    let val = u128::from(delta) * u128::from(snapshot.tsc_to_system_mul);

    // `tsc_shift` is validated in `setup` to have a magnitude less than 64.
    let res = if snapshot.tsc_shift >= 32 {
        val << ((snapshot.tsc_shift - 32) as u32)
    } else {
        val >> ((32 - snapshot.tsc_shift) as u32)
    };

    u64::try_from(res).unwrap_or(u64::MAX)
}
