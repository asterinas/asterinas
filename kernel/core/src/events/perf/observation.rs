// SPDX-License-Identifier: MPL-2.0

//! Conservative observation deadlines for modular hardware counters.

// A platform assumption, not an architectural limit: supported systems must
// retire fewer than 1024 instructions/cycles per nanosecond. In particular,
// instructions can retire several times faster than the clock frequency.
const MAX_INCREMENTS_PER_NS: u128 = 1024;

pub(super) fn interval_is_safe(width_bits: u8, last_ns: u64, now_ns: u64) -> bool {
    if !(32..=64).contains(&width_bits) {
        return false;
    }
    let max_gap_ns = (1u128 << width_bits) / MAX_INCREMENTS_PER_NS;
    now_ns
        .checked_sub(last_ns)
        .is_some_and(|gap| u128::from(gap) < max_gap_ns)
}

#[cfg(any(test, ktest))]
mod tests {
    #[cfg(ktest)]
    use ostd::prelude::ktest;

    use super::*;

    #[cfg_attr(ktest, ktest)]
    #[cfg_attr(not(ktest), test)]
    fn full_wrap_deadline_and_clock_regression_fail_closed() {
        let deadline = (1u64 << 32) / 1024;
        assert!(interval_is_safe(32, 10, 10 + deadline - 1));
        assert!(!interval_is_safe(32, 10, 10 + deadline));
        assert!(!interval_is_safe(32, 10, 9));
    }

    #[cfg_attr(ktest, ktest)]
    #[cfg_attr(not(ktest), test)]
    fn sixty_four_bit_width_and_invalid_metadata() {
        assert!(interval_is_safe(64, 0, 1_000_000_000));
        for width in [0, 31, 65, 255] {
            assert!(!interval_is_safe(width, 0, 0));
        }
    }
}
