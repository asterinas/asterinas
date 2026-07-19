// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use aster_block::BLOCK_SIZE;
use bitvec::{
    order::Lsb0,
    slice::BitSlice,
    view::{AsBits, AsMutBits},
};

use crate::prelude::*;

/// A disk I/O-friendly bitmap for ID management (e.g., block or inode IDs).
///
/// An ID bitmap has the same size as a block, i.e., `BLOCK_SIZE`.
/// Each bit in the bitmap represents one ID:
/// bit 0 at the i-th position of the bitmap means the i-th ID is free
/// and bit 1 means the ID is in use.
/// As such, the bitmap can contain contain at most `BLOCK_SIZE` * 8 bits/IDs.
#[derive(Clone)]
pub struct IdBitmap {
    buf: Box<[u8]>,
    first_available_id: u16,
    len: u16,
}

impl IdBitmap {
    /// Creates a new ID bitmap out of a given buffer, whose first `len`-bits represent valid IDs.
    ///
    /// # Panics
    ///
    /// This method panics if `len` is greater than [`IdBitmap::capacity()`].
    pub fn from_buf(buf: Box<[u8]>, len: u16) -> Self {
        assert!(len <= Self::capacity());
        let mut bitmap = Self {
            buf,
            first_available_id: 0,
            len,
        };

        let bit_slice = bitmap.bit_slice();
        bitmap.first_available_id = (0..len).find(|&i| !bit_slice[i as usize]).unwrap_or(len);
        bitmap
    }

    /// Returns the length of the ID bitmap, i.e., the maximum number of IDs.
    #[expect(unused)]
    pub const fn len(&self) -> u16 {
        self.len
    }

    /// Returns the capacity of the ID bitmap.
    ///
    /// The capacity is the size of the underlying buffer in bits.
    pub const fn capacity() -> u16 {
        BLOCK_SIZE as u16 * 8
    }

    /// Returns a reference to the underlying buffer of `BLOCK_SIZE` bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    fn bit_slice(&self) -> &BitSlice<u8, Lsb0> {
        &self.buf.as_bits()[..self.len as usize]
    }

    fn bit_slice_mut(&mut self) -> &mut BitSlice<u8, Lsb0> {
        &mut self.buf.as_mut_bits()[..self.len as usize]
    }

    /// Returns true if the `id` is allocated.
    ///
    /// # Panics
    ///
    /// If the `id` is out of bounds, this method will panic.
    pub fn is_allocated(&self, id: u16) -> bool {
        self.bit_slice()[id as usize]
    }

    /// Allocates and returns a new `id`.
    ///
    /// If allocation is not possible, it returns `None`.
    pub fn alloc(&mut self) -> Option<u16> {
        if self.first_available_id < self.len {
            let id = self.first_available_id;
            self.bit_slice_mut().set(id as usize, true);

            let bit_slice = self.bit_slice();
            self.first_available_id = (id + 1..self.len)
                .find(|&i| !bit_slice[i as usize])
                .unwrap_or(self.len);

            Some(id)
        } else {
            None
        }
    }

    /// Allocates a consecutive range of new `id`s.
    ///
    /// The `count` is the number of consecutive `id`s to allocate. If it is 0, return `None`.
    ///
    /// If allocation is not possible, it returns `None`.
    pub fn alloc_consecutive(&mut self, count: u16) -> Option<Range<u16>> {
        if count == 0 {
            return None;
        }

        let end = self.first_available_id.checked_add(count)?;
        if end > self.len {
            return None;
        }

        // Scan the bitmap from the position `first_available_id`
        // for the first `count` number of consecutive 0's.
        let allocated_range = {
            // Invariance: all bits within `curr_range` are 0's.
            let bit_slice = self.bit_slice();
            let mut curr_range = self.first_available_id..self.first_available_id + 1;
            while curr_range.len() < count as usize && curr_range.end < self.len {
                if !bit_slice[curr_range.end as usize] {
                    curr_range.end += 1;
                } else {
                    curr_range = curr_range.end + 1..curr_range.end + 1;
                }
            }

            if curr_range.len() < count as usize {
                return None;
            }

            curr_range
        };

        // Set every bit to 1 within the allocated range.
        let bit_slice_mut = self.bit_slice_mut();
        for id in allocated_range.clone() {
            bit_slice_mut.set(id as usize, true);
        }

        // In case we need to update `first_available_id`.
        let bit_slice = self.bit_slice();
        if bit_slice[self.first_available_id as usize] {
            self.first_available_id = (allocated_range.end..self.len)
                .find(|&i| !bit_slice[i as usize])
                .map_or(self.len, |i| i);
        }

        Some(allocated_range)
    }

    /// Releases the allocated `id`.
    ///
    /// # Panics
    ///
    /// If the `id` is out of bounds, this method will panic.
    pub fn free(&mut self, id: u16) {
        debug_assert!(self.bit_slice()[id as usize]);

        self.bit_slice_mut().set(id as usize, false);
        if id < self.first_available_id {
            self.first_available_id = id;
        }
    }

    /// Releases the consecutive range of allocated `id`s.
    ///
    /// # Panics
    ///
    /// If the `range` is out of bounds, this method will panic.
    pub fn free_consecutive(&mut self, range: Range<u16>) {
        if range.is_empty() {
            return;
        }

        let range_start = range.start;
        let bit_slice_mut = self.bit_slice_mut();
        for id in range {
            debug_assert!(bit_slice_mut[id as usize]);
            bit_slice_mut.set(id as usize, false);
        }

        if range_start < self.first_available_id {
            self.first_available_id = range_start
        }
    }
}

impl Debug for IdBitmap {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("IdBitMap")
            .field("len", &self.len)
            .field("first_available_id", &self.first_available_id)
            .finish()
    }
}

#[cfg(ktest)]
mod test {
    use alloc::vec;

    use aster_block::BLOCK_SIZE;
    use ostd::prelude::ktest;

    use super::IdBitmap;

    #[ktest]
    fn bitmap_alloc_out_of_bounds() {
        let buf = vec![0; BLOCK_SIZE].into_boxed_slice();

        let capacity = BLOCK_SIZE as u16 * 8;
        let mut bitmap = IdBitmap::from_buf(buf, capacity);

        for _ in 0..capacity {
            assert!(bitmap.alloc().is_some());
        }

        // Allocating one more ID should fail since the
        // bitmap's `first_available_id` + `count` is out of bounds.
        assert!(bitmap.alloc_consecutive(1).is_none());
    }
}
