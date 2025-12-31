// SPDX-License-Identifier: MPL-2.0

#![cfg_attr(not(test), no_std)]

/// An extension trait for Rust integer types, including `u8`, `u16`, `u32`,
/// `u64`, and `usize`, to provide methods to make integers aligned to a
/// power of two.
pub trait AlignExt: Sized {
    /// Returns to the smallest number that is greater than or equal to
    /// `self` and is a multiple of the given power of two.
    ///
    /// # Examples
    ///
    /// ```
    /// use crate::align_ext::AlignExt;
    /// assert_eq!(12usize.align_up(2), 12);
    /// assert_eq!(12usize.align_up(4), 12);
    /// assert_eq!(12usize.align_up(8), 16);
    /// assert_eq!(12usize.align_up(16), 16);
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if:
    ///  -`power_of_two` is not a power of two that is greater than or
    ///    equal to 2.
    ///  - the calculation overflows because `self` is too large.
    fn align_up(self, power_of_two: Self) -> Self;

    /// Returns to the smallest number that is greater than or equal to
    /// `self` and is a multiple of the given power of two.
    ///
    /// # Examples
    ///
    /// ```
    /// use crate::align_ext::AlignExt;
    /// assert_eq!(12usize.checked_align_up(2), Some(12));
    /// assert_eq!(12usize.checked_align_up(16), Some(16));
    /// assert_eq!(usize::MAX.checked_align_up(8), None);
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if `power_of_two` is not a power of two that is greater than or
    /// equal to 2.
    fn checked_align_up(self, power_of_two: Self) -> Option<Self>;

    /// Returns to the greatest number that is smaller than or equal to
    /// `self` and is a multiple of the given power of two.
    ///
    /// # Examples
    ///
    /// ```
    /// use crate::align_ext::AlignExt;
    /// assert_eq!(12usize.align_down(2), 12);
    /// assert_eq!(12usize.align_down(4), 12);
    /// assert_eq!(12usize.align_down(8), 8);
    /// assert_eq!(12usize.align_down(16), 0);
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if `power_of_two` is not a power of two that is greater than or
    /// equal to 2.
    fn align_down(self, power_of_two: Self) -> Self;
}

macro_rules! impl_align_ext {
    ($( $uint_type:ty ),+,) => {
        $(
            impl AlignExt for $uint_type {
                #[inline]
                fn align_up(self, align: Self) -> Self {
                    assert!(align.is_power_of_two() && align >= 2);
                    self.checked_add(align - 1).unwrap() & !(align - 1)
                }

                #[inline]
                fn checked_align_up(self, align: Self) -> Option<Self> {
                    assert!(align.is_power_of_two() && align >= 2);
                    Some(self.checked_add(align - 1)? & !(align - 1))
                }

                #[inline]
                fn align_down(self, align: Self) -> Self {
                    assert!(align.is_power_of_two() && align >= 2);
                    self & !(align - 1)
                }
            }
        )*
    }
}

impl_align_ext! {
    u8,
    u16,
    u32,
    u64,
    usize,
}

#[cfg(test)]
mod test {
    use super::*;

    macro_rules! check {
        ($fn:ident, $num:expr, $align:expr, $expected:expr) => {
            let num_ = $num.iter();
            let align_ = $align.iter();
            let expected_ = $expected.iter();

            for ((n, a), e) in num_.zip(align_).zip(expected_) {
                assert_eq!(n.$fn(*a), *e);
            }
        };
    }

    #[test]
    fn align_up() {
        check!(
            align_up,
            [0usize, 1, 2, 9, 15, 21, 32, 47, 50],
            [2usize, 2, 2, 2, 4, 4, 8, 8, 8],
            [0usize, 2, 2, 10, 16, 24, 32, 48, 56]
        );
    }

    #[test]
    fn checked_align_up() {
        check!(
            checked_align_up,
            [0usize, 1, 2, usize::MAX - 1, usize::MAX - 7],
            [2usize, 2, 2, 8, 8],
            [Some(0usize), Some(2), Some(2), None, Some(usize::MAX - 7)]
        );
    }

    #[test]
    fn align_down() {
        check!(
            align_down,
            [0usize, 1, 2, 9, 15, 21, 32, 47, 50],
            [2usize, 2, 2, 2, 4, 4, 8, 8, 8],
            [0usize, 0, 2, 8, 12, 20, 32, 40, 48]
        );
    }
}
