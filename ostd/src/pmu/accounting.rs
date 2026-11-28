// SPDX-License-Identifier: MPL-2.0

//! Counter width masking and modular wrap-around arithmetic.

/// The maximum bit width of a hardware performance counter.
const MAX_COUNTER_WIDTH_BITS: u8 = 64;

/// Returns a bitmask for a hardware counter of `width_bits` bits.
pub(super) const fn mask(width_bits: u8) -> u64 {
    if width_bits >= MAX_COUNTER_WIDTH_BITS {
        u64::MAX
    } else {
        (1u64 << width_bits) - 1
    }
}

/// Accumulates one hardware counter observation into `total`.
///
/// The caller must sample the counter before a full `2^width_bits` wrap occurs.
/// Returns `None` if the 64-bit software accumulator overflows.
pub(super) fn accumulate(total: &mut u64, last: &mut u64, raw: u64, width_bits: u8) -> Option<u64> {
    let counter_mask = mask(width_bits);
    let masked_raw = raw & counter_mask;
    let delta = masked_raw.wrapping_sub(*last) & counter_mask;

    *total = total.checked_add(delta)?;
    *last = masked_raw;
    Some(*total)
}
