// SPDX-License-Identifier: MPL-2.0

use core::mem;

use aster_time::NANOS_PER_SECOND;
use ostd::timer::TIMER_FREQ;
use spin::Once;

/// Returns the numerator and denominator of the ratio R:
///
///     R = 10^9 (ns in a sec) / TSC clock frequency
fn tsc_factors() -> (u64, u64) {
    static FACTORS: Once<(u64, u64)> = Once::new();
    *FACTORS.call_once(|| {
        let freq = ostd::arch::tsc_freq();
        assert_ne!(freq, 0);
        let mut a = 1_000_000_000;
        let mut b = freq;
        if a < b {
            mem::swap(&mut a, &mut b);
        }
        while a > 1 && b > 1 {
            let t = a;
            a = b;
            b = t % b;
        }

        let gcd = if a <= 1 { b } else { a };
        (1_000_000_000 / gcd, freq / gcd)
    })
}

/// The base time slice allocated for every thread, measured in nanoseconds.
pub const BASE_SLICE_NS: u64 = 700_000;

/// The duration between ticks, measured in nanoseconds.
pub const TICK_PERIOD_NS: u64 = (NANOS_PER_SECOND as u64 + TIMER_FREQ / 2) / TIMER_FREQ;

fn consts() -> (u64, u64) {
    static CONSTS: Once<(u64, u64)> = Once::new();
    *CONSTS.call_once(|| {
        let (a, b) = tsc_factors();
        (BASE_SLICE_NS * b / a, TICK_PERIOD_NS * b / a)
    })
}

/// Returns the base time slice allocated for every thread, measured in TSC clock units.
pub fn base_slice_clocks() -> u64 {
    consts().0
}

/// Returns the tick duration, measured in TSC clock units.
pub fn tick_period_clocks() -> u64 {
    consts().1
}
