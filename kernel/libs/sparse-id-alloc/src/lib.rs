// SPDX-License-Identifier: MPL-2.0

//! Sparse u32 ID allocators over configurable ranges.
//!
//! [`SparseIdAlloc`] allocates the smallest available ID in a range.
//! [`CyclicIdAlloc`] advances a cursor and only reuses lower IDs after wrapping.

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
        self.alloc_at_or_after(self.min)
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

    /// Allocates the smallest available ID greater than or equal to `start`.
    fn alloc_at_or_after(&mut self, start: u32) -> Option<u32> {
        let start = start.max(self.min);
        if start > self.max {
            return None;
        }

        let previous = self
            .allocated_runs
            .range(..=start)
            .next_back()
            .map(|(&run_start, &run_end)| (run_start, run_end));
        let id = match previous {
            Some((_, run_end)) if start <= run_end => run_end.checked_add(1)?,
            _ => start,
        };
        if id > self.max {
            return None;
        }

        self.mark_allocated(id, previous);
        Some(id)
    }

    /// Marks an available ID as allocated and merges adjacent allocated runs.
    fn mark_allocated(&mut self, id: u32, previous: Option<(u32, u32)>) {
        debug_assert!(previous.is_none_or(|(_, end)| id > end));

        let previous_start = previous
            .filter(|&(_, end)| end.checked_add(1) == Some(id))
            .map(|(start, _)| start);
        let next = id
            .checked_add(1)
            .and_then(|next_id| self.allocated_runs.remove_entry(&next_id));

        match (previous_start, next) {
            (Some(start), Some((_, end))) => {
                *self.allocated_runs.get_mut(&start).unwrap() = end;
            }
            (Some(start), None) => {
                *self.allocated_runs.get_mut(&start).unwrap() = id;
            }
            (None, Some((_, end))) => {
                self.allocated_runs.insert(id, end);
            }
            (None, None) => {
                self.allocated_runs.insert(id, id);
            }
        }
    }
}

/// A sparse allocator that searches for free IDs from a moving cursor.
///
/// IDs are first allocated from `initial_min` upwards. Once the cursor passes
/// `recycle_min`, a search that reaches `max` wraps back to `recycle_min`, so IDs
/// below that value are reserved after the initial allocation pass.
///
/// This follows the allocation policy of Linux's `idr_alloc_cyclic`.
/// Reference: <https://github.com/torvalds/linux/blob/v6.16/lib/idr.c#L96-L136>.
#[derive(Clone, Debug)]
pub struct CyclicIdAlloc {
    allocated: SparseIdAlloc,
    initial_min: u32,
    recycle_min: u32,
    next: u64,
}

impl CyclicIdAlloc {
    /// Creates an allocator over the inclusive range `[initial_min, max]`.
    ///
    /// After the initial allocation pass, searches wrap to `recycle_min`.
    ///
    /// # Panics
    ///
    /// Panics unless `initial_min <= recycle_min <= max`.
    pub const fn new(initial_min: u32, recycle_min: u32, max: u32) -> Self {
        assert!(
            initial_min <= recycle_min,
            "initial_min must be <= recycle_min"
        );
        assert!(recycle_min <= max, "recycle_min must be <= max");
        Self {
            allocated: SparseIdAlloc::new(initial_min, max),
            initial_min,
            recycle_min,
            next: initial_min as u64,
        }
    }

    /// Allocates the next available ID, wrapping after reaching the maximum.
    ///
    /// Returns `None` when every reusable ID is allocated.
    pub fn alloc(&mut self) -> Option<u32> {
        let wrap_min = if self.next > u64::from(self.recycle_min) {
            self.recycle_min
        } else {
            self.initial_min
        };

        let next_id = u32::try_from(self.next)
            .ok()
            .and_then(|next| self.allocated.alloc_at_or_after(next));
        let id = match next_id {
            Some(id) => id,
            None if self.next > u64::from(wrap_min) => {
                self.allocated.alloc_at_or_after(wrap_min)?
            }
            None => return None,
        };

        self.next = u64::from(id) + 1;
        Some(id)
    }

    /// Releases an allocated ID.
    ///
    /// # Panics
    ///
    /// Panics if the ID is not currently allocated.
    pub fn free(&mut self, id: u32) {
        self.allocated.free(id);
    }
}

#[cfg(test)]
mod test {
    use super::{CyclicIdAlloc, SparseIdAlloc};

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

    #[test]
    fn cyclic_alloc_reserves_low_ids_after_first_pass() {
        let mut a = CyclicIdAlloc::new(1, 3, 5);
        assert_eq!(a.alloc(), Some(1));
        assert_eq!(a.alloc(), Some(2));
        a.free(1);

        assert_eq!(a.alloc(), Some(3));
        assert_eq!(a.alloc(), Some(4));
        assert_eq!(a.alloc(), Some(5));
        assert_eq!(a.alloc(), None);
    }

    #[test]
    fn cyclic_alloc_wraps_to_recycle_min() {
        let mut a = CyclicIdAlloc::new(1, 3, 5);
        for expected in 1..=5 {
            assert_eq!(a.alloc(), Some(expected));
        }

        a.free(4);
        a.free(2);
        assert_eq!(a.alloc(), Some(4));
        assert_eq!(a.alloc(), None);
    }

    #[test]
    fn cyclic_alloc_reuses_gaps_after_the_cursor() {
        let mut a = CyclicIdAlloc::new(1, 1, 5);
        for expected in 1..=4 {
            assert_eq!(a.alloc(), Some(expected));
        }

        a.free(2);
        assert_eq!(a.alloc(), Some(5));
        assert_eq!(a.alloc(), Some(2));
    }

    #[test]
    fn cyclic_alloc_supports_u32_max() {
        let mut a = CyclicIdAlloc::new(u32::MAX, u32::MAX, u32::MAX);
        assert_eq!(a.alloc(), Some(u32::MAX));
        assert_eq!(a.alloc(), None);
        a.free(u32::MAX);
        assert_eq!(a.alloc(), Some(u32::MAX));
    }
}
