// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

/// Calculates the [difference] of two [`Range`]s, i.e., `a - b`.
///
/// This method will return 0, 1, or 2 ranges. All returned ranges are
/// guaranteed to be non-empty and non-overlapping. The returned ranges
/// will be sorted in ascending order.
///
/// [difference]: https://en.wikipedia.org/wiki/Set_(mathematics)#Set_difference
pub(crate) fn range_difference<T: Ord + Copy>(
    a: &Range<T>,
    b: &Range<T>,
) -> impl Iterator<Item = Range<T>> + use<T> {
    use core::cmp::{max, min};

    let r = if b.is_empty() {
        [a.clone(), b.clone()]
    } else {
        [a.start..min(a.end, b.start), max(a.start, b.end)..a.end]
    };

    r.into_iter().filter(|v| !v.is_empty())
}

/// Calculates the intersection of two [`Range`]s.
#[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
pub(crate) fn range_intersection<T: Ord + Copy>(a: &Range<T>, b: &Range<T>) -> Option<Range<T>> {
    let start = a.start.max(b.start);
    let end = a.end.min(b.end);
    (start < end).then_some(start..end)
}

#[cfg(ktest)]
#[expect(clippy::single_range_in_vec_init)]
mod test {
    use super::*;
    use crate::prelude::ktest;

    #[track_caller]
    fn assert_range_difference<const N: usize>(
        a: Range<usize>,
        b: Range<usize>,
        expected: [Range<usize>; N],
    ) {
        let mut res = range_difference(&a, &b);
        expected
            .into_iter()
            .for_each(|val| assert_eq!(res.next(), Some(val)));
        assert!(res.next().is_none());
    }

    #[ktest]
    fn range_difference_contained() {
        assert_range_difference(0..10, 3..7, [0..3, 7..10]);
    }
    #[ktest]
    fn range_difference_all_same() {
        assert_range_difference(0..10, 0..10, []);
    }
    #[ktest]
    fn range_difference_left_same() {
        assert_range_difference(0..10, 0..5, [5..10]);
    }
    #[ktest]
    fn range_difference_right_same() {
        assert_range_difference(0..10, 5..10, [0..5]);
    }
    #[ktest]
    fn range_difference_b_empty() {
        assert_range_difference(0..10, 0..0, [0..10]);
    }
    #[ktest]
    fn range_difference_a_empty() {
        assert_range_difference(0..0, 0..10, []);
    }
    #[ktest]
    fn range_difference_all_empty() {
        assert_range_difference(0..0, 0..0, []);
    }
    #[ktest]
    fn range_difference_left_intersected() {
        assert_range_difference(5..10, 0..6, [6..10]);
    }
    #[ktest]
    fn range_difference_right_intersected() {
        assert_range_difference(5..10, 6..12, [5..6]);
    }

    #[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
    #[ktest]
    fn range_intersection_overlapping() {
        assert_eq!(range_intersection(&(0..10), &(5..15)), Some(5..10));
    }
    #[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
    #[ktest]
    fn range_intersection_contained() {
        assert_eq!(range_intersection(&(0..10), &(3..7)), Some(3..7));
    }
    #[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
    #[ktest]
    fn range_intersection_disjoint() {
        assert_eq!(range_intersection(&(0..5), &(5..10)), None);
        assert_eq!(range_intersection(&(6..10), &(0..5)), None);
    }
    #[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
    #[ktest]
    fn range_intersection_empty() {
        assert_eq!(range_intersection(&(3..3), &(0..10)), None);
        assert_eq!(range_intersection(&(0..10), &(3..3)), None);
    }
}
