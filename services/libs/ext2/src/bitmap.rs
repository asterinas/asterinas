use crate::prelude::*;

use bitvec::prelude::*;

/// Bitmap.
/// The true bit means the id is allocated and vice versa.
#[derive(Clone)]
pub struct BitMap {
    bit_vec: BitVec<u8>,
    first_available_id: usize,
}

impl BitMap {
    /// Constructs a new bitmap from a slice of `u8` bytes and a bit length.
    ///
    /// The bit_len should not exceed the bit length of the slice.
    pub fn from_bytes_with_bit_len(slice: &[u8], bit_len: usize) -> Result<Self> {
        let bit_vec = {
            if bit_len > slice.len() * 8 {
                return Err(Error::BadBitMap);
            }
            let mut bit_vec = BitVec::from_slice(&slice[..bit_len.div_ceil(8)]);
            bit_vec.truncate(bit_len);
            bit_vec
        };

        let first_available_id = (0..bit_vec.len())
            .find(|&i| !bit_vec[i])
            .map_or(bit_vec.len(), |i| i);

        Ok(Self {
            bit_vec,
            first_available_id,
        })
    }

    /// Allocates and returns an id.
    ///
    /// Returns None if can not allocate.
    pub fn alloc(&mut self) -> Option<usize> {
        if self.first_available_id < self.bit_vec.len() {
            let id = self.first_available_id;
            self.bit_vec.set(id, true);
            self.first_available_id = (id + 1..self.bit_vec.len())
                .find(|&i| !self.bit_vec[i])
                .map_or(self.bit_vec.len(), |i| i);
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

        self.bit_vec.set(id, false);
        if id < self.first_available_id {
            self.first_available_id = id;
        }
    }

    /// Returns true is the id is allocated.
    ///
    /// This panics if the id is out of bounds.
    pub fn is_allocated(&self, id: usize) -> bool {
        self.bit_vec[id]
    }

    /// Views the bitmap as a slice of `u8` bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.bit_vec.as_raw_slice()
    }

    /// Views the bitmap as a mutable slice of `u8` bytes.
    pub fn as_mut_bytes(&mut self) -> &mut [u8] {
        self.bit_vec.as_raw_mut_slice()
    }
}

impl Debug for BitMap {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("BitMap")
            .field("len", &self.bit_vec.len())
            .field("first_available_id", &self.first_available_id)
            .finish()
    }
}
