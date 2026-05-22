// SPDX-License-Identifier: MPL-2.0

//! A sparse u32 ID allocator over a configurable range.
//!
//! The [`SparseIdAlloc`] type allocates the smallest available ID in an
//! inclusive `u32` range and supports returning IDs later.

#![cfg_attr(not(test), no_std)]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::collections::BTreeMap;

/// A sparse `u32` ID allocator over a configurable `[min, max]` range.
///
/// # Allocation policy
///
/// Always returns the smallest unallocated ID in the range, or `None`
/// when every ID is in use.
#[derive(Clone, Debug)]
pub struct SparseIdAlloc {
    min: u32,
    max: u32,
    /// Maximal runs of allocated IDs: start -> inclusive end.
    /// Runs are disjoint and non-adjacent.
    allocated_runs: BTreeMap<u32, u32>,
}

impl SparseIdAlloc {
    /// Creates a new allocator that hands out IDs from the inclusive range
    /// `[min, max]`.
    ///
    /// # Panics
    ///
    /// Panics if `min > max`.
    pub const fn new(min: u32, max: u32) -> Self {
        assert!(min <= max, "min must be <= max");
        Self {
            min,
            max,
            allocated_runs: BTreeMap::new(),
        }
    }

    /// Allocates and returns a new ID from the configured range.
    ///
    /// Returns `None` when every ID in `[min, max]` is in use.
    pub fn alloc(&mut self) -> Option<u32> {
        let first = self
            .allocated_runs
            .first_key_value()
            .map(|(&start, &end)| (start, end));

        match first {
            // The range's lowest IDs are allocated; the smallest free ID
            // is just past the first run (unless the range is exhausted).
            Some((start, end)) if start == self.min => {
                if end == self.max {
                    return None;
                }

                let id = end + 1;
                let new_end = id
                    .checked_add(1)
                    .and_then(|next_start| self.allocated_runs.remove(&next_start))
                    .unwrap_or(id);
                *self.allocated_runs.get_mut(&start).unwrap() = new_end;
                Some(id)
            }
            // `min` itself is free.
            _ => {
                let id = self.min;
                let end = id
                    .checked_add(1)
                    .and_then(|next_start| self.allocated_runs.remove(&next_start))
                    .unwrap_or(id);
                self.allocated_runs.insert(id, end);
                Some(id)
            }
        }
    }

    /// Releases the given ID.
    ///
    /// # Panics
    ///
    /// Panics if `id` is not currently allocated (out of range, never
    /// issued, or already freed).
    pub fn free(&mut self, id: u32) {
        let run = self
            .allocated_runs
            .range(..=id)
            .next_back()
            .map(|(&start, &end)| (start, end));
        let Some((start, end)) = run.filter(|&(_, end)| id <= end) else {
            panic!("free({id}) of unallocated ID");
        };

        if id == start {
            self.allocated_runs.remove(&start);
        } else {
            *self.allocated_runs.get_mut(&start).unwrap() = id - 1;
        }
        if id < end {
            self.allocated_runs.insert(id + 1, end);
        }
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
    fn sparse_alloc_reuses_freed_ids_immediately() {
        let mut a = SparseIdAlloc::new(1, u32::MAX);
        for _ in 0..3 {
            let _ = a.alloc();
        }
        a.free(3);
        a.free(1);
        a.free(2);
        assert_eq!(a.alloc(), Some(1));
        assert_eq!(a.alloc(), Some(2));
        assert_eq!(a.alloc(), Some(3));
    }

    #[test]
    fn sparse_alloc_reuses_smallest_gap_first() {
        let mut a = SparseIdAlloc::new(1, 4);
        for _ in 0..4 {
            let _ = a.alloc();
        }
        assert_eq!(a.alloc(), None);
        a.free(4);
        a.free(2);
        assert_eq!(a.alloc(), Some(2));
        assert_eq!(a.alloc(), Some(4));
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
