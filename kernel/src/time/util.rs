// SPDX-License-Identifier: MPL-2.0

#![expect(non_camel_case_types)]

use core::time::Duration;

use ostd::timer::{Jiffies, TIMER_FREQ};

use crate::time::NSEC_PER_SEC;

pub type clock_t = i64;

/// The user-space clock tick frequency exposed by Linux ABIs such as `times(2)`.
pub const USER_HZ: u64 = 100;

/// Converts a duration into kernel clock ticks.
pub fn duration_to_jiffies(duration: Duration) -> Jiffies {
    const NSEC_PER_JIFFY: u64 = NSEC_PER_SEC as u64 / TIMER_FREQ;
    const { assert!((NSEC_PER_SEC as u64).is_multiple_of(TIMER_FREQ)) };

    let sec_jiffies = duration.as_secs().saturating_mul(TIMER_FREQ);
    let subsec_jiffies = u64::from(duration.subsec_nanos()) / NSEC_PER_JIFFY;
    Jiffies::new(sec_jiffies.saturating_add(subsec_jiffies))
}

/// Converts kernel jiffies into the `clock_t` unit observed by user space.
pub fn jiffies_to_clock_t(jiffies: Jiffies) -> clock_t {
    let clock_ticks = jiffies.as_u64().saturating_mul(USER_HZ) / TIMER_FREQ;
    clock_ticks.min(clock_t::MAX as u64) as clock_t
}

/// Converts a duration directly into the `clock_t` unit observed by user space.
pub fn duration_to_clock_t(duration: Duration) -> clock_t {
    jiffies_to_clock_t(duration_to_jiffies(duration))
}
