// SPDX-License-Identifier: MPL-2.0

//! This module provides an abstraction `Coeff` to server for efficient and accurate calculation
//! of fraction multiplication.

use core::ops::Mul;

/// A `Coeff` is used to do a fraction multiplication operation with an unsigned integer.
/// It can achieve accurate and efficient calculation and avoid numeric overflow at the same time.
///
/// # Example
///
/// Let's say we want to multiply a fraction (23456 / 56789) with the target integer `a`,
/// which will be no larger than `1_000_000_000`, we can use the following code snippets
/// to get an accurate result.
///
/// ```
/// let a = input();
/// let coeff = Coeff::new(23456, 56789, 1_000_000_000);
/// let result = coeff * a;
/// ```
///
/// # How it works
/// `Coeff` is used in the calculation of a fraction value multiplied by an integer.
/// Here is a simple example of such calculation:
///
/// ```rust
/// let result = (a / b) * c;
/// ```
///
/// In this equation, `a`, `b`, `c` and `result` are all integers. To acquire a more precise result, we will
/// generally calculate `a * c` first and then divide the multiplication result with `b`.
/// However, this simple calculation above has two complications:
/// - The calculation of `a * c` may overflow if they are too large.
/// - The division operation is much more expensive than integer multiplication, which can easily create performance bottlenecks.
///
/// `Coeff` is implemented to address these two issues. It can be used to replace the fraction in this calculation.
/// For example, a `Coeff` generated from (a / b) can modify the calculation above to ensure that:
///
/// ```
/// coeff * c ~= (a / b) * c
/// ```
///
/// In principle, `Coeff` actually turns the multiplication and division into a combination of multiplication and bit operation.
/// When creating a `Coeff`, it needs to know the numerator and denominator of the represented fraction
/// and the max multiplier it will be multiplied by. Then, a `mult` and a `shift` will be chosen to achieve the replacement of calculation.
/// Taking the previous calculation as an example again, `coeff * c` will turn into `mult * c >> shift`, ensuring that:
///
/// ```
/// mult * c >> shift ~= (a / b) * c
/// ```
///
/// and
///  
/// `mult * c` will not result in numeric overflow (i.e., `mult * c` will stay below MAX_U64).
///
/// This is how `Coeff` achieves accuracy and efficiency at the same time.
#[derive(Debug, Copy, Clone)]
pub struct Coeff {
    mult: u32,
    shift: u32,
    max_multiplier: u64,
}

impl Coeff {
    /// Create a new coeff, which is essentially equivalent to （`numerator` / `denominator`) when being multiplied to an integer;
    /// Here users should make sure the multiplied integer should not be larger than `max_multiplier`.
    pub fn new(numerator: u64, denominator: u64, max_multiplier: u64) -> Self {
        let mut shift_acc: u32 = 32;
        // Too large `max_multiplier` will make the generated coeff imprecise
        debug_assert!(max_multiplier < (1 << 40));
        let mut tmp = max_multiplier >> 32;
        // Counts the number of 0 in front of the `max_multiplier`.
        // `shift_acc` indicates the maximum number of bits `mult` can have.
        while tmp > 0 {
            tmp >>= 1;
            shift_acc -= 1;
        }

        // Try the `shift` from 32 to 0.
        let mut shift = 32;
        let mut mult = 0;
        while shift > 0 {
            mult = numerator << shift;
            mult += denominator / 2;
            mult /= denominator;
            if (mult >> shift_acc) == 0 {
                break;
            }
            shift -= 1;
        }
        Self {
            mult: mult as u32,
            shift,
            max_multiplier,
        }
    }

    /// Return the `mult` of the Coeff.
    /// Only used for the VdsoData and will be removed in the future.
    pub fn mult(&self) -> u32 {
        self.mult
    }

    /// Return the `shift` of the Coeff.
    /// Only used for the VdsoData and will be removed in the future.
    pub fn shift(&self) -> u32 {
        self.shift
    }
}

impl Mul<u64> for Coeff {
    type Output = u64;
    fn mul(self, rhs: u64) -> Self::Output {
        debug_assert!(rhs <= self.max_multiplier);
        (rhs * self.mult as u64) >> self.shift
    }
}

impl Mul<u32> for Coeff {
    type Output = u32;
    fn mul(self, rhs: u32) -> Self::Output {
        debug_assert!(rhs as u64 <= self.max_multiplier);
        ((rhs as u64 * self.mult as u64) >> self.shift) as u32
    }
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::*;

    use super::*;

    #[ktest]
    fn calculation() {
        let coeff = Coeff::new(23456, 56789, 1_000_000_000);
        assert!(coeff * 0_u64 == 0);
        assert!(coeff * 100_u64 == 100 * 23456 / 56789);
        assert!(coeff * 1_000_000_000_u64 == 1_000_000_000 * 23456 / 56789);
    }
}
