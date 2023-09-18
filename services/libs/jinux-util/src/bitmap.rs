use bitvec::prelude::BitVec;
use core::fmt::Debug;

/// A BitMap implemented by bit vec.
/// The true bit means the id is allocated and vice versa.
#[derive(Clone)]
pub struct BitMap {
    bitset: BitVec<u8>,
    first_available_id: usize,
}

impl BitMap {
    /// Constructs a new bitmap with the maximum capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        let mut bitset = BitVec::with_capacity(capacity);
        bitset.resize(capacity, false);
        Self {
            bitset,
            first_available_id: 0,
        }
    }

    /// Constructs a new bitmap from a slice of `u8` bytes and a bit length.
    ///
    /// The bit_len should not exceed the bit length of the slice.
    pub fn from_bytes_with_bit_len(slice: &[u8], bit_len: usize) -> Option<Self> {
        let bitset = {
            if bit_len > slice.len() * 8 {
                return None;
            }
            let mut bitset = BitVec::from_slice(&slice[..bit_len.div_ceil(8)]);
            bitset.truncate(bit_len);
            bitset
        };

        let first_available_id = (0..bitset.len())
            .find(|&i| !bitset[i])
            .map_or(bitset.len(), |i| i);

        Some(Self {
            bitset,
            first_available_id,
        })
    }

    /// Allocates and returns an id.
    ///
    /// Returns None if can not allocate.
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

    /// Frees the allocated id.
    ///
    /// This panics if the id is out of bounds.
    pub fn free(&mut self, id: usize) {
        debug_assert!(self.is_allocated(id));

        self.bitset.set(id, false);
        if id < self.first_available_id {
            self.first_available_id = id;
        }
    }

    /// Returns true is the id is allocated.
    ///
    /// This panics if the id is out of bounds.
    pub fn is_allocated(&self, id: usize) -> bool {
        self.bitset[id]
    }

    /// Views the bitmap as a slice of `u8` bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.bitset.as_raw_slice()
    }

    /// Views the bitmap as a mutable slice of `u8` bytes.
    pub fn as_mut_bytes(&mut self) -> &mut [u8] {
        self.bitset.as_raw_mut_slice()
    }
}

impl Debug for BitMap {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("BitMap")
            .field("len", &self.bitset.len())
            .field("first_available_id", &self.first_available_id)
            .finish()
    }
}
