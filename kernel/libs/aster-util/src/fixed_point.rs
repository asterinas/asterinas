// SPDX-License-Identifier: MPL-2.0

//! A lightweight fixed-point number implementation optimized for kernel use.
//!
//! This crate provides a minimal, safe fixed-point arithmetic implementation
//! designed specifically for kernel development where zero `unsafe` code usage
//! throughout the entire implementation.

use core::{
    fmt,
    ops::{Add, Div, Mul, Sub},
};

macro_rules! define_fixed_unsigned {
    ($name:ident, $raw:ty, $wide:ty) => {
        /// A generic fixed-point number with `FRAC_BITS` fractional bits.
        ///
        /// This type represents a non-negative real number using an unsigned
        /// integer for storage, with the lower `FRAC_BITS` representing the
        /// fractional part.
        ///
        /// **Standard arithmetic operations can overflow and wrap around.**
        /// This follows Rust's default integer overflow behavior.
        ///
        /// For safer arithmetic that prevents overflow, use the
        /// `saturating_*` methods.
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub struct $name<const FRAC_BITS: u32>($raw);

        impl<const FRAC_BITS: u32> $name<FRAC_BITS> {
            const FRAC_SCALE: $raw = {
                // Do not remove or rewrite the const expression below.
                // It implicitly prevents users from giving invalid values of
                // `FRAC_BITS` greater than or equal to the raw integer width
                // because doing so would cause integer overflow during const
                // evaluation.
                1 << FRAC_BITS
            };

            pub const ZERO: Self = Self(0);
            pub const ONE: Self = Self(Self::FRAC_SCALE);
            const MAX_INT: $raw = <$raw>::MAX >> FRAC_BITS;

            /// Creates a fixed-point number from an integer.
            ///
            /// If the value is too large to be represented, it will saturate
            /// at the maximum representable value.
            pub const fn saturating_from_num(val: $raw) -> Self {
                if val > Self::MAX_INT {
                    Self(<$raw>::MAX)
                } else {
                    Self(val << FRAC_BITS)
                }
            }

            /// Creates a fixed-point number from raw bits.
            pub const fn from_raw(raw: $raw) -> Self {
                Self(raw)
            }

            /// Returns the raw underlying fixed-point value.
            pub const fn raw(self) -> $raw {
                self.0
            }

            /// Adds two fixed-point numbers, saturating on overflow.
            pub const fn saturating_add(self, other: Self) -> Self {
                Self(self.0.saturating_add(other.0))
            }

            /// Subtracts two fixed-point numbers, saturating on underflow.
            pub const fn saturating_sub(self, other: Self) -> Self {
                Self(self.0.saturating_sub(other.0))
            }

            /// Multiplies two fixed-point numbers, saturating on overflow.
            pub const fn saturating_mul(self, other: Self) -> Self {
                let result = (self.0 as $wide * other.0 as $wide) >> FRAC_BITS;
                Self(if result > <$raw>::MAX as $wide {
                    <$raw>::MAX
                } else {
                    result as $raw
                })
            }

            /// Divides two fixed-point numbers, saturating on overflow.
            ///
            /// Returns `None` if division by zero is attempted.
            pub const fn saturating_div(self, other: Self) -> Option<Self> {
                if other.0 == 0 {
                    return None;
                }

                let result = ((self.0 as $wide) << FRAC_BITS) / other.0 as $wide;
                Some(Self(if result > <$raw>::MAX as $wide {
                    <$raw>::MAX
                } else {
                    result as $raw
                }))
            }
        }

        impl<const FRAC_BITS: u32> Add for $name<FRAC_BITS> {
            type Output = Self;

            fn add(self, rhs: Self) -> Self::Output {
                Self(self.0 + rhs.0)
            }
        }

        impl<const FRAC_BITS: u32> Sub for $name<FRAC_BITS> {
            type Output = Self;

            fn sub(self, rhs: Self) -> Self::Output {
                Self(self.0 - rhs.0)
            }
        }

        impl<const FRAC_BITS: u32> Mul for $name<FRAC_BITS> {
            type Output = Self;

            fn mul(self, rhs: Self) -> Self::Output {
                let result = (self.0 as $wide * rhs.0 as $wide) >> FRAC_BITS;
                debug_assert!(
                    result <= <$raw>::MAX as $wide,
                    "attempt to multiply with overflow"
                );
                Self(result as $raw)
            }
        }

        impl<const FRAC_BITS: u32> Div for $name<FRAC_BITS> {
            type Output = Self;

            fn div(self, rhs: Self) -> Self::Output {
                let result = ((self.0 as $wide) << FRAC_BITS) / rhs.0 as $wide;
                debug_assert!(
                    result <= <$raw>::MAX as $wide,
                    "attempt to divide with overflow"
                );
                Self(result as $raw)
            }
        }

        impl<const FRAC_BITS: u32> fmt::Display for $name<FRAC_BITS> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    f,
                    "{}.{:>03}",
                    self.0 >> FRAC_BITS,
                    ((self.0 % Self::FRAC_SCALE) as $wide) * 1000 / Self::FRAC_SCALE as $wide
                )?;
                Ok(())
            }
        }
    };
}

define_fixed_unsigned!(FixedU32, u32, u64);
define_fixed_unsigned!(FixedU64, u64, u128);

impl<const FROM_FRAC_BITS: u32, const TO_FRAC_BITS: u32> From<FixedU32<FROM_FRAC_BITS>>
    for FixedU64<TO_FRAC_BITS>
{
    fn from(value: FixedU32<FROM_FRAC_BITS>) -> Self {
        const {
            assert!(TO_FRAC_BITS >= FROM_FRAC_BITS);
            assert!(u64::BITS - TO_FRAC_BITS >= u32::BITS - FROM_FRAC_BITS);
        }

        Self::from_raw((value.raw() as u64) << (TO_FRAC_BITS - FROM_FRAC_BITS))
    }
}

#[cfg(ktest)]
mod tests {
    extern crate alloc;

    use alloc::format;

    use ostd::prelude::*;

    use super::*;

    type FixedU32_8 = FixedU32<8>;
    type FixedU32_16 = FixedU32<16>;

    #[ktest]
    fn creation_methods() {
        // Test `saturating_from_num` with normal values
        let normal = FixedU32_8::saturating_from_num(42);
        assert_eq!(normal.raw(), 42 << 8);

        // Test `saturating_from_num` with overflow.
        let max_int = u32::MAX >> 8; // Maximum integer for FixedU32_8
        let at_limit = FixedU32_8::saturating_from_num(max_int);
        let over_limit = FixedU32_8::saturating_from_num(max_int + 1);

        assert_eq!(at_limit.raw(), max_int << 8);
        assert_eq!(over_limit.raw(), u32::MAX); // Should saturate

        // Test `from_raw`
        let half = FixedU32_8::from_raw(128); // 0.5 in 8.8 format
        assert_eq!(half.raw(), 128);
    }

    #[ktest]
    fn basic_arithmetic_methods() {
        let a = FixedU32_8::saturating_from_num(3); // 3.0
        let b = FixedU32_8::saturating_from_num(2); // 2.0

        // Test method-based arithmetic
        let sum = a + b;
        assert_eq!(sum.raw(), 5 << 8);

        let diff = a - b;
        assert_eq!(diff.raw(), 1 << 8);

        let prod = a * b;
        assert_eq!(prod.raw(), 6 << 8);

        let quotient = a / b;
        assert_eq!(quotient.raw(), 384);
    }

    #[ktest]
    fn saturating_arithmetic() {
        let max_val = FixedU32_8::from_raw(u32::MAX);
        let zero = FixedU32_8::ZERO;
        let one = FixedU32_8::saturating_from_num(1);

        let result = max_val.saturating_add(one);
        assert_eq!(result.raw(), u32::MAX);

        let result = zero.saturating_sub(one);
        assert_eq!(result.raw(), 0);

        let large = FixedU32_8::from_raw(u32::MAX / 2);
        let result = large.saturating_mul(FixedU32_8::saturating_from_num(3));
        assert_eq!(result.raw(), u32::MAX);

        let result = max_val.saturating_div(FixedU32_8::from_raw(1)).unwrap();
        assert_eq!(result.raw(), u32::MAX);
    }

    #[ktest]
    #[should_panic(expected = "attempt to divide by zero")]
    fn division_by_zero() {
        let a = FixedU32_8::saturating_from_num(5);
        let zero = FixedU32_8::ZERO;

        let _result = a / zero;
    }

    #[ktest]
    fn display_formatting() {
        // Test integer display
        let integer = FixedU32_8::saturating_from_num(42);
        let display_str = format!("{}", integer);
        assert_eq!(display_str, "42.000");

        // Test fractional display
        let fractional = FixedU32_8::from_raw(384); // 1.5
        let display_str = format!("{}", fractional);
        assert_eq!(display_str, "1.500");

        // Test zero
        let zero = FixedU32_8::ZERO;
        let display_str = format!("{}", zero);
        assert_eq!(display_str, "0.000");

        // Test with different precision
        let high_precision = FixedU32_16::from_raw(98304); // 1.5 in 16.16 format
        let display_str = format!("{}", high_precision);
        assert_eq!(display_str, "1.500");
    }

    #[ktest]
    fn fixed_u32_to_fixed_u64_conversion() {
        let value = FixedU32_8::from_raw(0x1234);
        let converted = FixedU64::<16>::from(value);

        assert_eq!(converted.raw(), 0x1234u64 << 8);
    }

    #[expect(clippy::eq_op)]
    #[ktest]
    fn edge_cases() {
        let zero = FixedU32_8::ZERO;
        let one = FixedU32_8::saturating_from_num(1);
        let val = FixedU32_8::saturating_from_num(42);

        // Zero multiplication
        assert_eq!(zero * val, zero);
        assert_eq!(val * zero, zero);

        // One multiplication (identity)
        assert_eq!(one * val, val);
        assert_eq!(val * one, val);

        // Self subtraction
        assert_eq!(val - val, zero);

        // Self division
        let result = val.div(val);
        assert_eq!(result.raw(), one.raw());
    }
}
