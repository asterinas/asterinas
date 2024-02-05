// SPDX-License-Identifier: MPL-2.0

use super::{constants::MIN_STD_SIG_NUM, sig_num::SigNum};

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub struct SigMask {
    bits: u64,
}

impl From<u64> for SigMask {
    fn from(bits: u64) -> Self {
        SigMask { bits }
    }
}

impl From<SigNum> for SigMask {
    fn from(sig_num: SigNum) -> Self {
        let idx = SigMask::num_to_idx(sig_num);
        let bits = 1u64 << idx;
        SigMask { bits }
    }
}

impl SigMask {
    pub fn new_empty() -> Self {
        SigMask::from(0u64)
    }

    pub fn new_full() -> Self {
        SigMask::from(!0u64)
    }

    pub const fn as_u64(&self) -> u64 {
        self.bits
    }

    pub const fn empty(&self) -> bool {
        self.bits == 0
    }

    pub const fn full(&self) -> bool {
        self.bits == !0
    }

    pub fn block(&mut self, block_sets: u64) {
        self.bits |= block_sets;
    }

    pub fn unblock(&mut self, unblock_sets: u64) {
        self.bits &= !unblock_sets;
    }

    pub fn set(&mut self, new_set: u64) {
        self.bits = new_set;
    }

    pub fn count(&self) -> usize {
        self.bits.count_ones() as usize
    }

    pub fn contains(&self, signum: SigNum) -> bool {
        let idx = Self::num_to_idx(signum);
        (self.bits & (1_u64 << idx)) != 0
    }

    fn num_to_idx(num: SigNum) -> usize {
        (num.as_u8() - MIN_STD_SIG_NUM) as usize
    }

    pub fn remove_signal(&mut self, signum: SigNum) {
        let idx = Self::num_to_idx(signum);
        self.bits &= !(1_u64 << idx);
    }

    pub fn add_signal(&mut self, signum: SigNum) {
        let idx = Self::num_to_idx(signum);
        self.bits |= 1_u64 << idx;
    }
}
