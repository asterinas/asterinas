// SPDX-License-Identifier: MPL-2.0

use core::fmt::{self};
use core::sync::atomic::{AtomicU64, Ordering::Relaxed};

use align_ext::AlignExt;

use crate::prelude::*;

/// A fixed number of bits taht can be safely shared between threads.
pub struct AtomicBits {
    num_bits: usize,
    u64s: Box<[AtomicU64]>,
}

impl AtomicBits {
    /// Create a given number of bit 0s.
    pub fn new_zeroes(num_bits: usize) -> Self {
        Self::new(0, num_bits)
    }

    /// Create a given number of bit 1s.
    pub fn new_ones(num_bits: usize) -> Self {
        Self::new(!0, num_bits)
    }

    fn new(u64_val: u64, num_bits: usize) -> Self {
        let num_u64s = num_bits.align_up(64) / 64;
        let u64s = {
            let mut u64s = Vec::with_capacity(num_u64s);
            for _ in 0..num_u64s {
                u64s.push(AtomicU64::new(u64_val));
            }
            u64s.into_boxed_slice()
        };
        Self { num_bits, u64s }
    }

    /// Returns the length in bits.
    pub fn len(&self) -> usize {
        self.num_bits
    }

    /// Get the bit at a given position.
    pub fn get(&self, index: usize) -> bool {
        assert!(index < self.num_bits);
        let i = index / 64;
        let j = index % 64;
        // Safety. Variable i is in range as variable index is in range.
        let u64_atomic = unsafe { self.u64s.get_unchecked(i) };
        (u64_atomic.load(Relaxed) & 1 << j) != 0
    }

    /// Set the bit at a given position.
    pub fn set(&self, index: usize, new_bit: bool) {
        assert!(index < self.num_bits);
        let i = index / 64;
        let j = index % 64;
        // Safety. Variable i is in range as variable index is in range.
        let u64_atomic = unsafe { self.u64s.get_unchecked(i) };
        if new_bit {
            u64_atomic.fetch_or(1 << j, Relaxed);
        } else {
            u64_atomic.fetch_and(!(1 << j), Relaxed);
        }
    }

    /// Clear all the bits.
    pub fn clear(&self) {
        todo!()
    }

    /// Are all bits ones.
    pub fn is_full(&self) -> bool {
        self.match_pattern(!0)
    }

    /// Are all bits zeroes.
    pub fn is_empty(&self) -> bool {
        self.match_pattern(0)
    }

    fn match_pattern(&self, pattern: u64) -> bool {
        todo!()
    }

    /// Get an iterator for the bits.
    pub fn iter(&self) -> Iter<'_> {
        Iter::new(self)
    }

    /// Get an iterator that gives the positions of all 1s in the bits.
    pub fn iter_ones(&self) -> OnesIter<'_> {
        OnesIter::new(self)
    }

    /// Get an iterator that gives the positions of all 0s in the bits.
    pub fn iter_zeroes(&self) -> ZeroesIter<'_> {
        ZeroesIter::new(self)
    }
}

/// An iterator that accesses the bits of an `AtomicBits`.
pub struct Iter<'a> {
    bits: &'a AtomicBits,
    bit_i: usize,
}

impl<'a> Iter<'a> {
    fn new(bits: &'a AtomicBits) -> Self {
        Self { bits, bit_i: 0 }
    }
}

impl<'a> Iterator for Iter<'a> {
    type Item = bool;

    fn next(&mut self) -> Option<bool> {
        if self.bit_i < self.bits.len() {
            let bit = self.bits.get(self.bit_i);
            self.bit_i += 1;
            Some(bit)
        } else {
            None
        }
    }
}

/// An iterator that returns the positions of 1s in an `AtomicBits`.
pub struct OnesIter<'a> {
    bits: &'a AtomicBits,
    u64_idx: usize,
    u64_val: u64,
    num_garbage_bits_in_last_u64: u8,
}

impl<'a> OnesIter<'a> {
    fn new(bits: &'a AtomicBits) -> Self {
        let num_garbage_bits_in_last_u64 = {
            if bits.len() % 64 != 0 {
                64 - ((bits.len() % 64) as u8)
            } else {
                0
            }
        };
        let mut new_self = Self {
            bits,
            u64_idx: 0,
            u64_val: 0, // NOT initalized yet!
            num_garbage_bits_in_last_u64,
        };
        new_self.u64_val = new_self.get_u64_val(0);
        new_self
    }

    /// Get the u64 value at the given position, removing the garbage bits if any.
    fn get_u64_val(&self, idx: usize) -> u64 {
        let mut u64_val = self.bits.u64s[idx].load(Relaxed);
        // Clear the garbage bits, if any, in the last u64 so that they
        // won't affect the result of the iterator.
        if idx == self.bits.u64s.len() - 1 && self.num_garbage_bits_in_last_u64 > 0 {
            let num_valid_bits_in_last_u64 = 64 - self.num_garbage_bits_in_last_u64;
            let valid_bits_mask = (1 << num_valid_bits_in_last_u64) - 1;
            u64_val &= valid_bits_mask;
        }
        u64_val
    }
}

impl<'a> Iterator for OnesIter<'a> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        loop {
            if self.u64_idx >= self.bits.u64s.len() {
                return None;
            }

            let first_one_in_u64 = self.u64_val.trailing_zeros() as usize;
            if first_one_in_u64 < 64 {
                self.u64_val &= !(1 << first_one_in_u64);
                let one_pos = self.u64_idx * 64 + first_one_in_u64;
                return Some(one_pos);
            }

            self.u64_idx += 1;
            if self.u64_idx < self.bits.u64s.len() {
                self.u64_val = self.get_u64_val(self.u64_idx);
            }
        }
    }
}

/// An iterator that returns the positions of 0s in an `AtomicBits`.
pub struct ZeroesIter<'a> {
    bits: &'a AtomicBits,
    u64_idx: usize,
    u64_val: u64,
    num_garbage_bits_in_last_u64: u8,
}

impl<'a> ZeroesIter<'a> {
    fn new(bits: &'a AtomicBits) -> Self {
        let num_garbage_bits_in_last_u64 = {
            if bits.len() % 64 != 0 {
                64 - ((bits.len() % 64) as u8)
            } else {
                0
            }
        };
        let mut new_self = Self {
            bits,
            u64_idx: 0,
            u64_val: 0, // NOT initalized yet!
            num_garbage_bits_in_last_u64,
        };
        new_self.u64_val = new_self.get_u64_val(0);
        new_self
    }

    /// Get the u64 value at the given position, removing the garbage bits if any.
    fn get_u64_val(&self, idx: usize) -> u64 {
        let mut u64_val = self.bits.u64s[idx].load(Relaxed);
        // Set all garbage bits, if any, in the last u64 so that they
        // won't affect the result of the iterator.
        if idx == self.bits.u64s.len() - 1 && self.num_garbage_bits_in_last_u64 > 0 {
            let num_valid_bits_in_last_u64 = 64 - self.num_garbage_bits_in_last_u64;
            let garbage_bits_mask = !((1 << num_valid_bits_in_last_u64) - 1);
            u64_val |= garbage_bits_mask;
        }
        u64_val
    }
}

impl<'a> Iterator for ZeroesIter<'a> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        loop {
            if self.u64_idx >= self.bits.u64s.len() {
                return None;
            }

            let first_zero_in_u64 = self.u64_val.trailing_ones() as usize;
            if first_zero_in_u64 < 64 {
                self.u64_val |= 1 << first_zero_in_u64;
                let one_pos = self.u64_idx * 64 + first_zero_in_u64;
                return Some(one_pos);
            }

            self.u64_idx += 1;
            if self.u64_idx < self.bits.u64s.len() {
                self.u64_val = self.get_u64_val(self.u64_idx);
            }
        }
    }
}

impl Clone for AtomicBits {
    fn clone(&self) -> Self {
        let num_bits = self.num_bits;
        let num_u64s = self.u64s.len();
        let u64s = {
            let mut u64s = Vec::with_capacity(num_u64s);
            for u64_i in 0..num_u64s {
                let u64_val = self.u64s[u64_i].load(Relaxed);
                u64s.push(AtomicU64::new(u64_val));
            }
            u64s.into_boxed_slice()
        };
        Self { num_bits, u64s }
    }
}

impl fmt::Debug for AtomicBits {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "AtomicBits(")?;
        for bit in self.iter() {
            if bit {
                write!(f, "1")?;
            } else {
                write!(f, "0")?;
            }
        }
        write!(f, ")")
    }
}

#[if_cfg_ktest]
mod test {
    use super::*;

    #[ktest]
    fn new() {
        let bits = AtomicBits::new_zeroes(1);
        assert!(bits.len() == 1);

        let bits = AtomicBits::new_zeroes(128);
        assert!(bits.len() == 128);

        let bits = AtomicBits::new_ones(7);
        assert!(bits.len() == 7);

        let bits = AtomicBits::new_zeroes(65);
        assert!(bits.len() == 65);
    }

    #[ktest]
    fn set_get() {
        let bits = AtomicBits::new_zeroes(128);
        for i in 0..bits.len() {
            assert!(bits.get(i) == false);

            bits.set(i, true);
            assert!(bits.get(i) == true);

            bits.set(i, false);
            assert!(bits.get(i) == false);
        }

        let bits = AtomicBits::new_ones(128);
        for i in 0..bits.len() {
            assert!(bits.get(i) == true);

            bits.set(i, false);
            assert!(bits.get(i) == false);

            bits.set(i, true);
            assert!(bits.get(i) == true);
        }
    }

    #[ktest]
    fn iter_ones() {
        let bits = AtomicBits::new_zeroes(1);
        assert!(bits.iter_ones().count() == 0);
        let bits = AtomicBits::new_zeroes(400);
        assert!(bits.iter_ones().count() == 0);

        let bits = AtomicBits::new_ones(1);
        assert!(bits.iter_ones().count() == 1);
        let bits = AtomicBits::new_ones(24);
        assert!(bits.iter_ones().count() == 24);
        let bits = AtomicBits::new_ones(64);
        assert!(bits.iter_ones().count() == 64);
        let bits = AtomicBits::new_ones(77);
        assert!(bits.iter_ones().count() == 77);
        let bits = AtomicBits::new_ones(128);
        assert!(bits.iter_ones().count() == 128);

        let bits = AtomicBits::new_zeroes(8);
        bits.set(1, true);
        bits.set(3, true);
        bits.set(5, true);
        assert!(bits.iter_ones().count() == 3);
    }

    #[ktest]
    fn iter_zeroes() {
        let bits = AtomicBits::new_ones(1);
        assert!(bits.iter_zeroes().count() == 0);
        let bits = AtomicBits::new_ones(130);
        assert!(bits.iter_zeroes().count() == 0);

        let bits = AtomicBits::new_zeroes(1);
        assert!(bits.iter_zeroes().count() == 1);
        let bits = AtomicBits::new_zeroes(24);
        assert!(bits.iter_zeroes().count() == 24);
        let bits = AtomicBits::new_zeroes(64);
        assert!(bits.iter_zeroes().count() == 64);
        let bits = AtomicBits::new_zeroes(77);
        assert!(bits.iter_zeroes().count() == 77);
        let bits = AtomicBits::new_zeroes(128);
        assert!(bits.iter_zeroes().count() == 128);

        let bits = AtomicBits::new_ones(96);
        bits.set(1, false);
        bits.set(3, false);
        bits.set(5, false);
        bits.set(64, false);
        bits.set(76, false);
        assert!(bits.iter_zeroes().count() == 5);
    }

    #[ktest]
    fn iter() {
        let bits = AtomicBits::new_zeroes(7);
        assert!(bits.iter().all(|bit| bit == false));

        let bits = AtomicBits::new_ones(128);
        assert!(bits.iter().all(|bit| bit == true));
    }
}
