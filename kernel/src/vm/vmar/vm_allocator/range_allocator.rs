// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use align_ext::AlignExt;

use crate::{
    prelude::*,
    vm::vmar::{Interval, IntervalSet},
};

/// A simple range allocator that is not scalable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeAllocator {
    freelist: IntervalSet<Vaddr, FreeRange>,
}

/// Options for allocation.
///
/// Used for [`RangeAllocator::alloc`].
pub enum AllocOption {
    Specific(Range<Vaddr>),
    General { size: usize, align: usize },
}

impl RangeAllocator {
    pub fn new_empty() -> Self {
        Self {
            freelist: IntervalSet::new(),
        }
    }

    pub fn new(full_range: Range<Vaddr>) -> Self {
        let mut freelist = IntervalSet::new();
        freelist.insert(FreeRange::new(full_range));
        Self { freelist }
    }

    pub fn fork_from(&mut self, other: &Self) {
        self.freelist = other.freelist.clone();
    }

    pub fn alloc(
        &mut self,
        size: usize,
        align: usize,
        strategy: fn(&Self, usize, usize) -> Option<FoundFitRange>,
    ) -> Result<Vaddr> {
        if size == 0 {
            return_errno_with_message!(Errno::EINVAL, "cannot allocate zero-sized range");
        }
        if align == 0 || !align.is_power_of_two() {
            return_errno_with_message!(Errno::EINVAL, "alignment must be a power of two");
        }

        let FoundFitRange {
            free_start,
            alloc_start,
            alloc_end,
            free_end,
        } = strategy(self, size, align).ok_or(Error::with_message(
            Errno::ENOMEM,
            "out of virtual addresses",
        ))?;

        self.freelist
            .remove(&free_start)
            .expect("free range selected from iterable must exist");

        if free_start < alloc_start {
            self.freelist
                .insert(FreeRange::new(free_start..alloc_start));
        }
        if alloc_end < free_end {
            self.freelist.insert(FreeRange::new(alloc_end..free_end));
        }

        Ok(alloc_start)
    }

    /// Finds a free range that can fit the requested size and alignment quickly.
    ///
    /// This function allocates the highest address in the first found range to
    /// trade fragmentation for allocation speed.
    ///
    /// Can be used as the `strategy` parameter of [`Self::alloc`].
    pub fn find_top_fast(allocator: &Self, size: usize, align: usize) -> Option<FoundFitRange> {
        for free in allocator.freelist.iter() {
            if let Some(found) = free.find_fits(size, align) {
                return Some(found);
            }
        }

        None
    }

    /// Finds the topmost range that can fit the requested size and alignment.
    ///
    /// It reduces fragmentation but is slower than [`Self::find_top_fast`].
    ///
    /// Can be used as the `strategy` parameter of [`Self::alloc`].
    pub fn find_top_slow(allocator: &Self, size: usize, align: usize) -> Option<FoundFitRange> {
        for free in allocator.freelist.iter().rev() {
            if let Some(found) = free.find_fits(size, align) {
                return Some(found);
            }
        }

        None
    }

    pub fn alloc_specific(&mut self, range: Range<Vaddr>) -> Result<Vaddr> {
        if range.is_empty() {
            return_errno_with_message!(Errno::EINVAL, "cannot allocate empty range");
        }

        let Some(containing) = self
            .freelist
            .find_one(&range.start)
            .map(|free| free.range())
        else {
            return_errno_with_message!(Errno::ENOMEM, "requested range is not free");
        };

        if range.end > containing.end {
            return_errno_with_message!(Errno::ENOMEM, "requested range spans multiple free slots");
        }

        self.freelist
            .remove(&containing.start)
            .expect("containing range must exist");

        if containing.start < range.start {
            self.freelist
                .insert(FreeRange::new(containing.start..range.start));
        }
        if range.end < containing.end {
            self.freelist
                .insert(FreeRange::new(range.end..containing.end));
        }

        Ok(range.start)
    }

    /// # Panics
    ///
    /// Panics if the given range is already added.
    pub fn add_range_try_merge(&mut self, range: Range<Vaddr>) {
        if range.is_empty() {
            return;
        }

        let mut merged_start = range.start;
        let mut merged_end = range.end;

        if let Some(prev) = self.freelist.find_prev(&range.start) {
            let prev_range = prev.range();
            assert!(
                !range_overlaps(&prev_range, &range),
                "added range overlaps with existing free range"
            );
            if prev_range.end == range.start {
                self.freelist.remove(&prev_range.start);
                merged_start = prev_range.start;
            }
        }

        if let Some(next) = self.freelist.find_next(&range.start) {
            let next_range = next.range();
            assert!(
                !range_overlaps(&next_range, &range),
                "added range overlaps with existing free range"
            );
            if next_range.start == range.end {
                self.freelist.remove(&next_range.start);
                merged_end = next_range.end;
            }
        }

        self.freelist
            .insert(FreeRange::new(merged_start..merged_end));
    }

    /// Takes all free ranges overlapping with the given range.
    pub(super) fn take_free_ranges(
        &mut self,
        range: &Range<Vaddr>,
    ) -> impl Iterator<Item = FreeRange> {
        self.freelist.take(range)
    }

    /// Adds a free range without merging.
    pub(super) fn add_range_without_merge(&mut self, range: FreeRange) {
        self.freelist.insert(range);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FreeRange {
    block: Range<Vaddr>,
}

pub struct FoundFitRange {
    free_start: Vaddr,
    alloc_start: Vaddr,
    alloc_end: Vaddr,
    free_end: Vaddr,
}

impl FreeRange {
    const fn new(range: Range<Vaddr>) -> Self {
        Self { block: range }
    }

    /// Truncates the range into possibly three parts.
    ///
    /// Assuming we call `a.truncate(b)`, the three parts are:
    ///  - The part of `a` before `b`, if any;
    ///  - The part of `a` overlapping with `b`, if any;
    ///  - The part of `a` after `b`, if any.
    pub(super) fn truncate(
        self,
        range: Range<Vaddr>,
    ) -> (Option<Self>, Option<Self>, Option<Self>) {
        let mut before = None;
        let mut overlapping = None;
        let mut after = None;

        if range.start > self.block.start {
            before = Some(FreeRange::new(
                self.block.start..range.start.min(self.block.end),
            ));
        }

        let overlap_start = self.block.start.max(range.start);
        let overlap_end = self.block.end.min(range.end);
        if overlap_start < overlap_end {
            overlapping = Some(FreeRange::new(overlap_start..overlap_end));
        }

        if range.end < self.block.end {
            after = Some(FreeRange::new(
                range.end.max(self.block.start)..self.block.end,
            ));
        }

        (before, overlapping, after)
    }

    /// Find the topmost range that fits the given size and alignment.
    fn find_fits(&self, size: usize, align: usize) -> Option<FoundFitRange> {
        let start = self.block.end.checked_sub(size)?;
        let aligned_start = start.align_down(align);
        if aligned_start < self.block.start {
            return None;
        }
        let aligned_end = aligned_start + size;
        let found = FoundFitRange {
            free_start: self.block.start,
            alloc_start: aligned_start,
            alloc_end: aligned_end,
            free_end: self.block.end,
        };
        debug_assert!(found.free_start <= found.alloc_start);
        debug_assert!(found.alloc_end <= found.free_end);
        Some(found)
    }
}

impl Interval<Vaddr> for FreeRange {
    fn range(&self) -> Range<Vaddr> {
        self.block.clone()
    }
}

fn range_overlaps(a: &Range<Vaddr>, b: &Range<Vaddr>) -> bool {
    a.start < b.end && b.start < a.end
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::*;

    use super::*;

    #[ktest]
    fn range_overlaps_correct() {
        assert!(range_overlaps(&(0..10), &(5..15)));
        assert!(range_overlaps(&(0..10), &(0..1)));
        assert!(range_overlaps(&(5..15), &(10..15)));
        assert!(!range_overlaps(&(0..10), &(10..20)));
        assert!(!range_overlaps(&(15..25), &(5..15)));
    }

    #[ktest]
    fn free_range_truncate() {
        let (before, overlapping, after) = FreeRange::new(100..200).truncate(120..180);
        assert_eq!(before.unwrap().range(), 100..120);
        assert_eq!(overlapping.unwrap().range(), 120..180);
        assert_eq!(after.unwrap().range(), 180..200);

        let (before2, overlapping2, after2) = FreeRange::new(100..200).truncate(50..120);
        assert!(before2.is_none());
        assert_eq!(overlapping2.unwrap().range(), 100..120);
        assert_eq!(after2.unwrap().range(), 120..200);
    }

    #[ktest]
    fn alloc_succeeds() {
        let mut allocator = RangeAllocator::new(0..128);

        let first = allocator
            .alloc(16, 8, RangeAllocator::find_top_fast)
            .expect("initial alloc fails");
        assert!(first.is_multiple_of(8));

        let second = allocator
            .alloc(16, 32, RangeAllocator::find_top_fast)
            .expect("aligned allocation should succeed");
        assert!(!range_overlaps(
            &(first..first + 16),
            &(second..second + 16)
        ));
        assert!(second.is_multiple_of(32));

        let third = allocator
            .alloc(8, 16, RangeAllocator::find_top_fast)
            .expect("third alloc fails");
        assert!(!range_overlaps(&(first..first + 16), &(third..third + 8)));
        assert!(!range_overlaps(&(second..second + 16), &(third..third + 8)));
        assert!(third.is_multiple_of(16));

        let free_ranges_total_size: usize = allocator
            .freelist
            .iter()
            .map(|free| free.range().len())
            .sum();
        assert_eq!(free_ranges_total_size, 128 - (16 + 16 + 8));
    }

    #[ktest]
    fn alloc_fails() {
        let mut allocator = RangeAllocator::new(0..64);

        let err = allocator
            .alloc(0, 8, RangeAllocator::find_top_fast)
            .unwrap_err();
        assert_eq!(err.error(), Errno::EINVAL);

        let err = allocator
            .alloc(8, 3, RangeAllocator::find_top_fast)
            .unwrap_err();
        assert_eq!(err.error(), Errno::EINVAL);

        allocator
            .alloc(48, 8, RangeAllocator::find_top_fast)
            .expect("able to consume most space");
        let err = allocator
            .alloc(32, 8, RangeAllocator::find_top_fast)
            .unwrap_err();
        assert_eq!(err.error(), Errno::ENOMEM);
    }

    #[ktest]
    fn find_top_slow_find_topmost() {
        let mut allocator = RangeAllocator::new(0..128);

        let first = allocator
            .alloc(16, 8, RangeAllocator::find_top_slow)
            .expect("initial alloc fails");
        assert_eq!(first, 112);

        let second = allocator
            .alloc(16, 64, RangeAllocator::find_top_slow)
            .expect("aligned allocation should succeed");
        assert_eq!(second, 64);

        let third = allocator
            .alloc(8, 16, RangeAllocator::find_top_slow)
            .expect("third alloc fails");
        assert_eq!(third, 96);
    }

    #[ktest]
    fn alloc_specific_succeeds() {
        let mut allocator = RangeAllocator::new(0..128);
        let target = 32..64;

        let start = allocator
            .alloc_specific(target.clone())
            .expect("specific allocation should succeed");
        assert_eq!(start, target.start);

        let free_ranges: Vec<Range<Vaddr>> =
            allocator.freelist.iter().map(|free| free.range()).collect();
        assert_eq!(free_ranges, vec![0..32, 64..128]);
    }

    #[ktest]
    fn alloc_specific_fails() {
        let mut allocator = RangeAllocator::new(0..64);

        let err = allocator.alloc_specific(8..72).unwrap_err();
        assert_eq!(err.error(), Errno::ENOMEM);

        allocator
            .alloc_specific(0..16)
            .expect("initial specific allocation succeeds");
        let err = allocator.alloc_specific(0..16).unwrap_err();
        assert_eq!(err.error(), Errno::ENOMEM);
    }

    #[ktest]
    fn add_will_merge() {
        let mut allocator = RangeAllocator::new_empty();
        allocator.add_range_try_merge(0..32);
        allocator.add_range_try_merge(64..96);
        allocator.add_range_try_merge(32..64);

        let free_ranges: Vec<Range<Vaddr>> =
            allocator.freelist.iter().map(|free| free.range()).collect();
        assert_eq!(free_ranges, vec![0..96]);
    }

    #[ktest]
    #[should_panic]
    fn add_duplicate() {
        let mut allocator = RangeAllocator::new_empty();
        allocator.add_range_try_merge(0..32);
        allocator.add_range_try_merge(16..48);
    }
}
