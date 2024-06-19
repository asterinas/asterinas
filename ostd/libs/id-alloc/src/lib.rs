// SPDX-License-Identifier: MPL-2.0

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

use core::{fmt::Debug, ops::Range};

use bitvec::prelude::BitVec;

/// An id allocator implemented by the bitmap.
/// The true bit implies that the id is allocated, and vice versa.
#[derive(Clone)]
pub struct IdAlloc {
    bitset: BitVec<u8>,
    first_available_id: usize,
}

impl IdAlloc {
    /// Constructs a new id allocator with a maximum capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        let mut bitset = BitVec::with_capacity(capacity);
        bitset.resize(capacity, false);
        Self {
            bitset,
            first_available_id: 0,
        }
    }

    /// Constructs a new id allocator from a slice of `u8` bytes and a maximum capacity.
    ///
    /// The slice of `u8` bytes is the raw data of a bitmap.
    pub fn from_bytes_with_capacity(slice: &[u8], capacity: usize) -> Self {
        let bitset = if capacity > slice.len() * 8 {
            let mut bitset = BitVec::from_slice(slice);
            bitset.resize(capacity, false);
            bitset
        } else {
            let mut bitset = BitVec::from_slice(&slice[..capacity.div_ceil(8)]);
            bitset.truncate(capacity);
            bitset
        };

        let first_available_id = (0..bitset.len())
            .find(|&i| !bitset[i])
            .map_or(bitset.len(), |i| i);

        Self {
            bitset,
            first_available_id,
        }
    }

    /// Allocates and returns a new `id`.
    ///
    /// If allocation is not possible, it returns `None`.
    pub fn alloc(&mut self) -> Option<usize> {
        if self.first_available_id < self.bitset.len() {
            let id = self.first_available_id;
            self.bitset.set(id, true);
            self.first_available_id = (id + 1..self.bitset.len())
                .find(|&i| !self.bitset[i])
                .map_or(self.bitset.len(), |i| i);
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
    ///
    /// TODO: Choose a more efficient strategy.
    pub fn alloc_consecutive(&mut self, count: usize) -> Option<Range<usize>> {
        if count == 0 {
            return None;
        }

        // Scan the bitmap from the position `first_available_id`
        // for the first `count` number of consecutive 0's.
        let allocated_range = {
            // Invariance: all bits within `curr_range` are 0's
            let mut curr_range = self.first_available_id..self.first_available_id + 1;
            while curr_range.len() < count && curr_range.end < self.bitset.len() {
                if !self.is_allocated(curr_range.end) {
                    curr_range.end += 1;
                } else {
                    curr_range = curr_range.end + 1..curr_range.end + 1;
                }
            }

            if curr_range.len() < count {
                return None;
            }

            curr_range
        };

        // Set every bit to 1 within the allocated range
        for id in allocated_range.clone() {
            self.bitset.set(id, true);
        }

        // In case we need to update first_available_id
        if self.is_allocated(self.first_available_id) {
            self.first_available_id = (allocated_range.end..self.bitset.len())
                .find(|&i| !self.bitset[i])
                .map_or(self.bitset.len(), |i| i);
        }

        Some(allocated_range)
    }

    /// Releases the consecutive range of allocated `id`s.
    ///
    /// # Panics
    ///
    /// If the `range` is out of bounds, this method will panic.
    pub fn free_consecutive(&mut self, range: Range<usize>) {
        if range.is_empty() {
            return;
        }

        let range_start = range.start;
        for id in range {
            debug_assert!(self.is_allocated(id));
            self.bitset.set(id, false);
        }

        if range_start < self.first_available_id {
            self.first_available_id = range_start
        }
    }

    /// Releases the allocated `id`.
    ///
    /// # Panics
    ///
    /// If the `id` is out of bounds, this method will panic.
    pub fn free(&mut self, id: usize) {
        debug_assert!(self.is_allocated(id));

        self.bitset.set(id, false);
        if id < self.first_available_id {
            self.first_available_id = id;
        }
    }

    /// Allocate a specific ID.
    ///
    /// If the ID is already allocated, it returns `None`, otherwise it
    /// returns the allocated ID.
    ///
    /// # Panics
    ///
    /// If the `id` is out of bounds, this method will panic.
    pub fn alloc_specific(&mut self, id: usize) -> Option<usize> {
        if self.bitset[id] {
            return None;
        }
        self.bitset.set(id, true);
        if id == self.first_available_id {
            self.first_available_id = (id + 1..self.bitset.len())
                .find(|&i| !self.bitset[i])
                .map_or(self.bitset.len(), |i| i);
        }
        Some(id)
    }

    /// Returns true if the `id` is allocated.
    ///
    /// # Panics
    ///
    /// If the `id` is out of bounds, this method will panic.
    pub fn is_allocated(&self, id: usize) -> bool {
        self.bitset[id]
    }

    /// Views the id allocator as a slice of `u8` bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.bitset.as_raw_slice()
    }
}

impl Debug for IdAlloc {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("IdAlloc")
            .field("len", &self.bitset.len())
            .field("first_available_id", &self.first_available_id)
            .finish()
    }
}
