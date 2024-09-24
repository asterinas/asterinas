// SPDX-License-Identifier: MPL-2.0

//! Signal sets and atomic masks.
//!
//! A signal set is a bit-set of signals. A signal mask is a set of signals
//! that are blocked from delivery to a thread. An atomic signal mask
//! implementation is provided for shared access to signal masks.

use core::{
    fmt::LowerHex,
    ops,
    sync::atomic::{AtomicU64, Ordering},
};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;

use super::{constants::MIN_STD_SIG_NUM, sig_num::SigNum};
use crate::prelude::*;

/// A signal mask.
///
/// This is an alias to the [`SigSet`]. All the signal in the set are blocked
/// from the delivery to a thread.
pub type SigMask = SigSet;

/// A bit-set of signals.
///
/// Because that all the signal numbers are in the range of 1 to 64, casting
/// a signal set from `u64` to `SigSet` will always succeed.
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Pod)]
#[repr(C)]
pub struct SigSet {
    bits: u64,
}

impl From<SigNum> for SigSet {
    fn from(signum: SigNum) -> Self {
        let idx = signum.as_u8() - MIN_STD_SIG_NUM;
        Self { bits: 1_u64 << idx }
    }
}

impl From<u64> for SigSet {
    fn from(bits: u64) -> Self {
        SigSet { bits }
    }
}

impl From<SigSet> for u64 {
    fn from(set: SigSet) -> u64 {
        set.bits
    }
}

impl<T: Into<SigSet>> ops::BitAnd<T> for SigSet {
    type Output = Self;

    fn bitand(self, rhs: T) -> Self {
        SigSet {
            bits: self.bits & rhs.into().bits,
        }
    }
}

impl<T: Into<SigSet>> ops::BitAndAssign<T> for SigSet {
    fn bitand_assign(&mut self, rhs: T) {
        self.bits &= rhs.into().bits;
    }
}

impl<T: Into<SigSet>> ops::BitOr<T> for SigSet {
    type Output = Self;

    fn bitor(self, rhs: T) -> Self {
        SigSet {
            bits: self.bits | rhs.into().bits,
        }
    }
}

impl<T: Into<SigSet>> ops::BitOrAssign<T> for SigSet {
    fn bitor_assign(&mut self, rhs: T) {
        self.bits |= rhs.into().bits;
    }
}

#[allow(clippy::suspicious_arithmetic_impl)]
impl<T: Into<SigSet>> ops::Add<T> for SigSet {
    type Output = Self;

    fn add(self, rhs: T) -> Self {
        SigSet {
            bits: self.bits | rhs.into().bits,
        }
    }
}

#[allow(clippy::suspicious_op_assign_impl)]
impl<T: Into<SigSet>> ops::AddAssign<T> for SigSet {
    fn add_assign(&mut self, rhs: T) {
        self.bits |= rhs.into().bits;
    }
}

impl<T: Into<SigSet>> ops::Sub<T> for SigSet {
    type Output = Self;

    fn sub(self, rhs: T) -> Self {
        SigSet {
            bits: self.bits & !rhs.into().bits,
        }
    }
}

impl<T: Into<SigSet>> ops::SubAssign<T> for SigSet {
    fn sub_assign(&mut self, rhs: T) {
        self.bits &= !rhs.into().bits;
    }
}

impl SigSet {
    pub fn new_empty() -> Self {
        SigSet { bits: 0 }
    }

    pub fn new_full() -> Self {
        SigSet { bits: !0 }
    }

    pub const fn is_empty(&self) -> bool {
        self.bits == 0
    }

    pub const fn is_full(&self) -> bool {
        self.bits == !0
    }

    pub fn count(&self) -> usize {
        self.bits.count_ones() as usize
    }

    pub fn contains(&self, set: impl Into<Self>) -> bool {
        let set = set.into();
        self.bits & set.bits == set.bits
    }
}

// This is to allow hexadecimally formatting a `SigSet` when debug printing it.
impl LowerHex for SigSet {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        LowerHex::fmt(&self.bits, f) // delegate to u64's implementation
    }
}

/// An atomic signal mask.
///
/// This is an alias to the [`AtomicSigSet`]. All the signal in the set are
/// blocked from the delivery to a thread.
///
/// [`Relaxed`]: core::sync::atomic::Ordering::Relaxed
pub type AtomicSigMask = AtomicSigSet;

define_atomic_version_of_integer_like_type!(SigSet, {
    pub struct AtomicSigSet(AtomicU64);
});

impl From<SigSet> for AtomicSigSet {
    fn from(set: SigSet) -> Self {
        Self::new(set)
    }
}

impl AtomicSigSet {
    pub fn new_empty() -> Self {
        AtomicSigSet::new(0)
    }

    pub fn new_full() -> Self {
        AtomicSigSet::new(!0)
    }

    pub fn contains(&self, signals: impl Into<SigSet>, ordering: Ordering) -> bool {
        self.load(ordering).contains(signals.into())
    }
}
