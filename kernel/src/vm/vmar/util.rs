// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

/// Determines whether two ranges are intersected.
/// returns false if one of the ranges has a length of 0
pub fn is_intersected(range1: &Range<usize>, range2: &Range<usize>) -> bool {
    range1.start.max(range2.start) < range1.end.min(range2.end)
}

/// Gets the intersection range of two ranges.
/// The two ranges should be ensured to be intersected.
pub fn get_intersected_range(range1: &Range<usize>, range2: &Range<usize>) -> Range<usize> {
    debug_assert!(is_intersected(range1, range2));
    range1.start.max(range2.start)..range1.end.min(range2.end)
}
