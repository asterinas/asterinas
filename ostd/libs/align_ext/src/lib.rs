// SPDX-License-Identifier: MPL-2.0

#![cfg_attr(not(test), no_std)]

/// An extension trait for Rust integer types, including `u8`, `u16`, `u32`,
/// `u64`, and `usize`, to provide methods to make integers aligned to a
/// power of two.
pub trait AlignExt {
    /// Returns to the smallest number that is greater than or equal to
    /// `self` and is a multiple of the given power of two.
    ///
    /// The method panics if `power_of_two` is not a
    /// power of two or is smaller than 2 or the calculation overflows
    /// because `self` is too large.
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
    fn align_up(self, power_of_two: Self) -> Self;

    /// Returns to the greatest number that is smaller than or equal to
    /// `self` and is a multiple of the given power of two.
    ///
    /// The method panics if `power_of_two` is not a
    /// power of two or is smaller than 2 or the calculation overflows
    /// because `self` is too large. In release mode,
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

    #[test]
    fn test_align_up() {
        let input_ns = [0usize, 1, 2, 9, 15, 21, 32, 47, 50];
        let input_as = [2usize, 2, 2, 2, 4, 4, 8, 8, 8];
        let output_ns = [0usize, 2, 2, 10, 16, 24, 32, 48, 56];

        for i in 0..input_ns.len() {
            let n = input_ns[i];
            let a = input_as[i];
            let n2 = output_ns[i];
            assert!(n.align_up(a) == n2);
        }
    }

    #[test]
    fn test_align_down() {
        let input_ns = [0usize, 1, 2, 9, 15, 21, 32, 47, 50];
        let input_as = [2usize, 2, 2, 2, 4, 4, 8, 8, 8];
        let output_ns = [0usize, 0, 2, 8, 12, 20, 32, 40, 48];

        for i in 0..input_ns.len() {
            let n = input_ns[i];
            let a = input_as[i];
            let n2 = output_ns[i];
            assert!(n.align_down(a) == n2);
        }
    }
}
