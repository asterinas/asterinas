// SPDX-License-Identifier: MPL-2.0

pub mod range_alloc;

use core::ops::Range;

/// Asserts that a boolean expression is `true` at compile-time.
///
/// Rust provides [`const` blocks], which can be used flexibly within methods, but cannot be used
/// directly at the top level. This macro serves as a helper to perform compile-time assertions
/// outside of methods.
///
/// [`const` blocks]: https://doc.rust-lang.org/reference/expressions/block-expr.html#const-blocks
//
// TODO: Introduce `const_assert_eq!()` once `assert_eq!()` can be used in the `const` context.
#[macro_export]
macro_rules! const_assert {
    ($cond:expr $(,)?) => { const _: () = assert!($cond); };
    ($cond:expr, $($arg:tt)+) => { const _: () = assert!($cond, $($arg)*); };
}

/// A marker trait that represents a type has the same size as `T`.
///
/// # Safety
///
/// Types that implement `SameSizeAs<T>` must have the same size as `T`.
pub unsafe trait SameSizeAs<T> {}

/// Calculates the [difference] of two [`Range`]s, i.e., `a - b`.
///
/// This method will return 0, 1, or 2 ranges. All returned ranges are
/// guaranteed to be non-empty and non-overlapping. The returned ranges
/// will be sorted in ascending order.
///
/// [difference]: https://en.wikipedia.org/wiki/Set_(mathematics)#Set_difference
pub fn range_difference<T: Ord + Copy>(
    a: &Range<T>,
    b: &Range<T>,
) -> impl Iterator<Item = Range<T>> {
    use core::cmp::{max, min};

    let r = if b.is_empty() {
        [a.clone(), b.clone()]
    } else {
        [a.start..min(a.end, b.start), max(a.start, b.end)..a.end]
    };

    r.into_iter().filter(|v| !v.is_empty())
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
}
