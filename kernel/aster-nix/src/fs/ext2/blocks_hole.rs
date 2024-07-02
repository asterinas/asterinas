// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use core::ops::Range;

use bitvec::prelude::BitVec;

/// A blocks hole descriptor implemented by the `BitVec`.
///
/// The true bit implies that the block is a hole, and conversely.
pub(super) struct BlocksHoleDesc(BitVec);

impl BlocksHoleDesc {
    /// Constructs a blocks hole descriptor with initial size.
    ///
    /// The `initial_size` usually is the number of blocks for a file.
    pub fn new(initial_size: usize) -> Self {
        let mut bit_vec = BitVec::with_capacity(initial_size);
        bit_vec.resize(initial_size, false);
        Self(bit_vec)
    }

    /// Returns the size.
    pub fn size(&self) -> usize {
        self.0.len()
    }

    /// Resizes the blocks hole to a new size.
    ///
    /// If `new_size` is greater than current size, the new blocks are all marked as hole.
    pub fn resize(&mut self, new_size: usize) {
        self.0.resize(new_size, true);
    }

    /// Returns if the block `idx` is a hole.
    ///
    /// # Panics
    ///
    /// If the `idx` is out of bounds, this method will panic.
    pub fn is_hole(&self, idx: usize) -> bool {
        self.0[idx]
    }

    /// Marks the block `idx` as a hole.
    ///
    /// # Panics
    ///
    /// If the `idx` is out of bounds, this method will panic.
    pub fn set(&mut self, idx: usize) {
        self.0.set(idx, true);
    }

    /// Marks all blocks within the `range` as holes.
    ///
    /// # Panic
    ///
    /// If the `range` is out of bounds, this method will panic.
    pub fn set_range(&mut self, range: Range<usize>) {
        for idx in range {
            self.0.set(idx, true);
        }
    }

    /// Unmarks the block `idx` as a hole.
    ///
    /// # Panics
    ///
    /// If the `idx` is out of bounds, this method will panic.
    pub fn unset(&mut self, idx: usize) {
        self.0.set(idx, false);
    }

    /// Unmarks all blocks within the `range` as holes.
    ///
    /// # Panic
    ///
    /// If the `range` is out of bounds, this method will panic.
    pub fn unset_range(&mut self, range: Range<usize>) {
        for idx in range {
            self.0.set(idx, false);
        }
    }
}
