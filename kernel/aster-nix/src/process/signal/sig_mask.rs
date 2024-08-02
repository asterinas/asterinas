// SPDX-License-Identifier: MPL-2.0

use core::{
    ops,
    sync::atomic::{AtomicU64, Ordering},
};

use super::{constants::MIN_STD_SIG_NUM, sig_num::SigNum};
use crate::prelude::*;

/// A bit-set of signals.
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Pod)]
#[repr(C)]
pub struct SigSet {
    bits: u64,
}

impl From<SigNum> for SigSet {
    fn from(signum: SigNum) -> Self {
        let idx = Self::num_to_idx(signum);
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

impl ops::BitAnd for SigSet {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self {
        SigSet {
            bits: self.bits & rhs.bits,
        }
    }
}

impl ops::BitAndAssign for SigSet {
    fn bitand_assign(&mut self, rhs: Self) {
        self.bits &= rhs.bits;
    }
}

impl ops::BitOr for SigSet {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        SigSet {
            bits: self.bits | rhs.bits,
        }
    }
}

impl ops::BitOrAssign for SigSet {
    fn bitor_assign(&mut self, rhs: Self) {
        self.bits |= rhs.bits;
    }
}

impl ops::Sub for SigSet {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        SigSet {
            bits: self.bits & !rhs.bits,
        }
    }
}

impl ops::SubAssign for SigSet {
    fn sub_assign(&mut self, rhs: Self) {
        self.bits &= !rhs.bits;
    }
}

impl SigSet {
    pub fn new_empty() -> Self {
        SigSet { bits: 0 }
    }

    pub fn new_full() -> Self {
        SigSet { bits: !0 }
    }

    pub const fn as_u64(&self) -> u64 {
        self.bits
    }

    pub const fn is_empty(&self) -> bool {
        self.bits == 0
    }

    pub const fn is_full(&self) -> bool {
        self.bits == !0
    }

    pub fn reset(&mut self, new_set: u64) {
        self.bits = new_set;
    }

    pub fn count(&self) -> usize {
        self.bits.count_ones() as usize
    }

    pub fn contains(&self, signum: SigNum) -> bool {
        let idx = Self::num_to_idx(signum);
        (self.bits & (1_u64 << idx)) != 0
    }

    pub fn remove_signal(&mut self, signum: SigNum) {
        let idx = Self::num_to_idx(signum);
        self.bits &= !(1_u64 << idx);
    }

    pub fn add_signal(&mut self, signum: SigNum) {
        let idx = Self::num_to_idx(signum);
        self.bits |= 1_u64 << idx;
    }

    fn num_to_idx(num: SigNum) -> usize {
        (num.as_u8() - MIN_STD_SIG_NUM) as usize
    }
}

/// An atomic signal mask.
///
/// All operations to this signal uses the [`Relaxed`] ordering. So the precise
/// order of blocking and unblocking signals may not be consistent among
/// threads. Blocking and unblocking signals cannot fence out any critical
/// sections either.
///
/// [`Relaxed`]: core::sync::atomic::Ordering::Relaxed
pub struct AtomicSigMask(AtomicU64);

impl From<SigSet> for AtomicSigMask {
    fn from(set: SigSet) -> Self {
        AtomicSigMask(AtomicU64::new(set.bits))
    }
}

impl AtomicSigMask {
    pub fn new_empty() -> Self {
        AtomicSigMask(AtomicU64::new(0))
    }

    pub fn new_full() -> Self {
        AtomicSigMask(AtomicU64::new(!0))
    }

    pub fn load(&self, ordering: Ordering) -> SigSet {
        SigSet {
            bits: self.0.load(ordering),
        }
    }

    pub fn block(&self, mask: SigSet) {
        self.0.fetch_or(mask.bits, Ordering::Relaxed);
    }

    pub fn unblock(&self, mask: SigSet) {
        self.0.fetch_and(!mask.bits, Ordering::Relaxed);
    }

    pub fn reset(&self, mask: SigSet) {
        self.0.store(mask.bits, Ordering::Relaxed);
    }

    pub fn contains(&self, signum: SigNum) -> bool {
        SigSet {
            bits: self.0.load(Ordering::Relaxed),
        }
        .contains(signum)
    }
}
