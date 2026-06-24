// SPDX-License-Identifier: MPL-2.0

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::collections::BTreeSet;

/// A sparse `u32` ID allocator over a configurable `[min, max]` range.
///
/// # Allocation policy
///
/// IDs are issued in increasing order starting from `min`. Freed IDs
/// are not immediately reused; the allocator advances its watermark
/// past them so that every newly issued ID is greater than every
/// previously issued one while fresh IDs remain. Once the watermark
/// has covered the entire range, [`Self::alloc`] returns the smallest
/// unallocated ID in `[min, max]`, or `None` when every ID is in use.
#[derive(Clone, Debug)]
pub struct SparseIdAlloc {
    min: u32,
    max: u32,
    /// Next never-issued ID. `None` once the watermark has passed `max`.
    next_id: Option<u32>,
    allocated: BTreeSet<u32>,
}

impl SparseIdAlloc {
    /// Creates a new allocator that hands out IDs from the inclusive range
    /// `[min, max]`.
    ///
    /// # Panics
    ///
    /// Panics if `min > max`.
    pub fn new(min: u32, max: u32) -> Self {
        assert!(min <= max, "min ({min}) must be <= max ({max})");
        Self {
            min,
            max,
            next_id: Some(min),
            allocated: BTreeSet::new(),
        }
    }

    /// Allocates and returns a new ID from the configured range.
    ///
    /// Returns `None` when every ID in `[min, max]` is in use.
    pub fn alloc(&mut self) -> Option<u32> {
        if let Some(candidate) = self.next_id
            && candidate <= self.max
        {
            self.next_id = candidate.checked_add(1);
            self.allocated.insert(candidate);
            return Some(candidate);
        }
        self.next_id = None;

        let mut cursor = self.min;
        for &id in self.allocated.iter() {
            if id > cursor {
                break;
            }
            cursor = cursor.checked_add(1)?;
            if cursor > self.max {
                return None;
            }
        }
        self.allocated.insert(cursor);
        Some(cursor)
    }

    /// Releases the given ID.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if `id` is not currently allocated (out of
    /// range, never issued, or already freed). In release builds, such
    /// misuses are silent no-ops.
    pub fn free(&mut self, id: u32) {
        let removed = self.allocated.remove(&id);
        debug_assert!(removed, "free({id}) of unallocated ID");
    }
}

#[cfg(test)]
mod test {
    use super::SparseIdAlloc;

    #[test]
    fn sparse_alloc_is_monotonic_from_min() {
        let mut a = SparseIdAlloc::new(1, u32::MAX);
        assert_eq!(a.alloc(), Some(1));
        assert_eq!(a.alloc(), Some(2));
        assert_eq!(a.alloc(), Some(3));
    }

    #[test]
    fn sparse_alloc_advances_watermark_past_freed_ids() {
        let mut a = SparseIdAlloc::new(1, u32::MAX);
        for _ in 0..3 {
            let _ = a.alloc();
        }
        a.free(3);
        a.free(1);
        a.free(2);
        // Freed IDs are not reused while fresh IDs are still available.
        assert_eq!(a.alloc(), Some(4));
        assert_eq!(a.alloc(), Some(5));
        assert_eq!(a.alloc(), Some(6));
    }

    #[test]
    fn sparse_alloc_reuses_smallest_gap_after_exhaustion() {
        let mut a = SparseIdAlloc::new(1, 3);
        for _ in 0..3 {
            let _ = a.alloc();
        }
        assert_eq!(a.alloc(), None);
        a.free(3);
        a.free(1);
        // Smallest unallocated comes first; the larger gap is filled last.
        assert_eq!(a.alloc(), Some(1));
        assert_eq!(a.alloc(), Some(3));
        assert_eq!(a.alloc(), None);
    }

    #[test]
    fn sparse_alloc_handles_single_id_range() {
        let mut a = SparseIdAlloc::new(u32::MAX, u32::MAX);
        assert_eq!(a.alloc(), Some(u32::MAX));
        assert_eq!(a.alloc(), None);
    }

    #[test]
    #[should_panic]
    fn sparse_alloc_panics_when_min_greater_than_max() {
        let _ = SparseIdAlloc::new(5, 4);
    }

    #[test]
    #[should_panic(expected = "unallocated")]
    fn sparse_alloc_panics_on_free_never_issued() {
        let mut a = SparseIdAlloc::new(1, 100);
        let _ = a.alloc();
        a.free(5);
    }

    #[test]
    #[should_panic(expected = "unallocated")]
    fn sparse_alloc_panics_on_double_free() {
        let mut a = SparseIdAlloc::new(1, 100);
        let id = a.alloc().unwrap();
        a.free(id);
        a.free(id);
    }
}
