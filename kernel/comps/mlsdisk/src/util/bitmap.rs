// SPDX-License-Identifier: MPL-2.0

use core::ops::Index;

use bittle::{Bits, BitsMut};
use serde::{Deserialize, Serialize};

use crate::prelude::*;

/// A compact array of bits.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BitMap {
    bits: Vec<u64>,
    nbits: usize,
}

impl BitMap {
    /// The one bit represents `true`.
    const ONE: bool = true;

    /// The zero bit represents `false`.
    const ZERO: bool = false;

    /// Create a new `BitMap` by repeating the `value` for the desired length.
    pub fn repeat(value: bool, nbits: usize) -> Self {
        let vec_len = nbits.div_ceil(64);
        let mut bits = Vec::with_capacity(vec_len);
        if value == Self::ONE {
            bits.resize(vec_len, !0u64);
        } else {
            bits.resize(vec_len, 0u64);
        }

        // Set the unused bits in the last u64 with zero.
        if nbits % 64 != 0 {
            bits[vec_len - 1]
                .iter_ones()
                .filter(|index| (*index as usize) >= nbits % 64)
                .for_each(|index| bits[vec_len - 1].clear_bit(index));
        }

        Self { bits, nbits }
    }

    /// Return the total number of bits.
    pub fn len(&self) -> usize {
        self.nbits
    }

    fn check_index(&self, index: usize) {
        if index >= self.len() {
            panic!(
                "bitmap index {} is out of range, total bits {}",
                index, self.nbits,
            );
        }
    }

    /// Test if the given bit is set.
    ///
    /// Return `true` if the given bit is one bit.
    ///
    /// # Panics
    ///
    /// The `index` must be within the total number of bits. Otherwise, this method panics.
    pub fn test_bit(&self, index: usize) -> bool {
        self.check_index(index);
        self.bits.test_bit(index as _)
    }

    /// Set the given bit with one bit.
    ///
    /// # Panics
    ///
    /// The `index` must be within the total number of bits. Otherwise, this method panics.
    pub fn set_bit(&mut self, index: usize) {
        self.check_index(index);
        self.bits.set_bit(index as _);
    }

    /// Clear the given bit with zero bit.
    ///
    /// # Panics
    ///
    /// The `index` must be within the total number of bits. Otherwise, this method panics.
    pub fn clear_bit(&mut self, index: usize) {
        self.check_index(index);
        self.bits.clear_bit(index as _)
    }

    /// Set the given bit with `value`.
    ///
    /// One bit is set for `true`, and zero bit for `false`.
    ///
    /// # Panics
    ///
    /// The `index` must be within the total number of bits. Otherwise, this method panics.
    pub fn set(&mut self, index: usize, value: bool) {
        if value == Self::ONE {
            self.set_bit(index);
        } else {
            self.clear_bit(index);
        }
    }

    fn bits_not_in_use(&self) -> usize {
        self.bits.len() * 64 - self.nbits
    }

    /// Get the number of one bits in the bitmap.
    pub fn count_ones(&self) -> usize {
        self.bits.count_ones() as _
    }

    /// Get the number of zero bits in the bitmap.
    pub fn count_zeros(&self) -> usize {
        let total_zeros = self.bits.count_zeros() as usize;
        total_zeros - self.bits_not_in_use()
    }

    /// Find the index of the first one bit, starting from the given index (inclusively).
    ///
    /// Return `None` if no one bit is found.
    ///
    /// # Panics
    ///
    /// The `from` index must be within the total number of bits. Otherwise, this method panics.
    pub fn first_one(&self, from: usize) -> Option<usize> {
        self.check_index(from);
        let first_u64_index = from / 64;

        self.bits[first_u64_index..]
            .iter_ones()
            .map(|index| first_u64_index * 64 + (index as usize))
            .find(|&index| index >= from)
    }

    /// Find `count` indexes of the first one bits, starting from the given index (inclusively).
    ///
    /// Return `None` if fewer than `count` one bits are found.
    ///
    /// # Panics
    ///
    /// The `from + count` index must be within the total number of bits. Otherwise, this method panics.
    pub fn first_ones(&self, from: usize, count: usize) -> Option<Vec<usize>> {
        self.check_index(from + count - 1);
        let first_u64_index = from / 64;

        let ones: Vec<_> = self.bits[first_u64_index..]
            .iter_ones()
            .map(|index| first_u64_index * 64 + (index as usize))
            .filter(|&index| index >= from)
            .take(count)
            .collect();
        if ones.len() == count {
            Some(ones)
        } else {
            None
        }
    }

    /// Find the index of the last one bit.
    ///
    /// Return `None` if no one bit is found.
    pub fn last_one(&self) -> Option<usize> {
        self.bits
            .iter_ones()
            .rev()
            .map(|index| index as usize)
            .next()
    }

    /// Find the index of the first zero bit, starting from the given index (inclusively).
    ///
    /// Return `None` if no zero bit is found.
    ///
    /// # Panics
    ///
    /// The `from` index must be within the total number of bits. Otherwise, this method panics.
    pub fn first_zero(&self, from: usize) -> Option<usize> {
        self.check_index(from);
        let first_u64_index = from / 64;

        self.bits[first_u64_index..]
            .iter_zeros()
            .map(|index| first_u64_index * 64 + (index as usize))
            .find(|&index| index >= from && index < self.len())
    }

    /// Find `count` indexes of the first zero bits, starting from the given index (inclusively).
    ///
    /// Return `None` if fewer than `count` zero bits are found.
    ///
    /// # Panics
    ///
    /// The `from + count` index must be within the total number of bits. Otherwise, this method panics.
    pub fn first_zeros(&self, from: usize, count: usize) -> Option<Vec<usize>> {
        self.check_index(from + count - 1);
        let first_u64_index = from / 64;

        let zeros: Vec<_> = self.bits[first_u64_index..]
            .iter_zeros()
            .map(|index| first_u64_index * 64 + (index as usize))
            .filter(|&index| index >= from && index < self.len())
            .take(count)
            .collect();
        if zeros.len() == count {
            Some(zeros)
        } else {
            None
        }
    }

    /// Find the index of the last zero bit.
    ///
    /// Return `None` if no zero bit is found.
    pub fn last_zero(&self) -> Option<usize> {
        self.bits
            .iter_zeros()
            .rev()
            .skip(self.bits_not_in_use())
            .map(|index| index as usize)
            .next()
    }
}

impl Index<usize> for BitMap {
    type Output = bool;

    fn index(&self, index: usize) -> &Self::Output {
        if self.test_bit(index) {
            &BitMap::ONE
        } else {
            &BitMap::ZERO
        }
    }
}

#[cfg(test)]
mod tests {
    use super::BitMap;

    #[test]
    fn all_true() {
        let bm = BitMap::repeat(true, 100);
        assert_eq!(bm.len(), 100);
        assert_eq!(bm.count_ones(), 100);
        assert_eq!(bm.count_zeros(), 0);
    }

    #[test]
    fn all_false() {
        let bm = BitMap::repeat(false, 100);
        assert_eq!(bm.len(), 100);
        assert_eq!(bm.count_ones(), 0);
        assert_eq!(bm.count_zeros(), 100);
    }

    #[test]
    fn bit_ops() {
        let mut bm = BitMap::repeat(false, 100);

        assert_eq!(bm.count_ones(), 0);

        bm.set_bit(32);
        assert_eq!(bm.count_ones(), 1);
        assert_eq!(bm.test_bit(32), true);

        bm.set(64, true);
        assert_eq!(bm.count_ones(), 2);
        assert_eq!(bm.test_bit(64), true);

        bm.clear_bit(32);
        assert_eq!(bm.count_ones(), 1);
        assert_eq!(bm.test_bit(32), false);

        bm.set(64, false);
        assert_eq!(bm.count_ones(), 0);
        assert_eq!(bm.test_bit(64), false);
    }

    #[test]
    fn find_first_last() {
        let mut bm = BitMap::repeat(false, 100);
        bm.set_bit(64);
        assert_eq!(bm.first_one(0), Some(64));
        assert_eq!(bm.first_one(64), Some(64));
        assert_eq!(bm.first_one(65), None);
        assert_eq!(bm.first_ones(0, 1), Some(vec![64]));
        assert_eq!(bm.first_ones(0, 2), None);
        assert_eq!(bm.last_one(), Some(64));

        let mut bm = BitMap::repeat(true, 100);
        bm.clear_bit(64);
        assert_eq!(bm.first_zero(0), Some(64));
        assert_eq!(bm.first_zero(64), Some(64));
        assert_eq!(bm.first_zero(65), None);
        assert_eq!(bm.first_zeros(0, 1), Some(vec![64]));
        assert_eq!(bm.first_zeros(0, 2), None);
        assert_eq!(bm.last_zero(), Some(64));
    }
}
