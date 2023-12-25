use bitvec::prelude::BitVec;

/// An id allocator with BitVec.
/// The true bit means the id is allocated and vice versa.
#[derive(Clone, Debug)]
pub struct IdAlloc {
    bitset: BitVec,
    first_available_id: usize,
}

impl IdAlloc {
    /// Constructs a new id allocator with the maximum capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        let mut bitset = BitVec::with_capacity(capacity);
        bitset.resize(capacity, false);
        Self {
            bitset,
            first_available_id: 0,
        }
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
}
