// SPDX-License-Identifier: MPL-2.0

//! Per-CPU allocator for virtual memory ranges.

use alloc::vec::Vec;
use core::{
    ops::Range,
    sync::atomic::{AtomicUsize, Ordering},
};

use align_ext::AlignExt;
use osdk_heap_allocator::{alloc_cpu_local, CpuLocalBox};
use ostd::{
    cpu::{all_cpus, num_cpus, CpuId, PinCurrentCpu},
    mm::Vaddr,
    sync::PreemptDisabled,
    task::disable_preempt,
    util::id_set::Id,
};

use super::{Interval, IntervalSet, VMAR_CAP_ADDR, VMAR_LOWEST_ADDR};
use crate::prelude::*;

/// The per-CPU allocator for virtual memory addresses.
pub struct PerCpuAllocator {
    /// Initially, the full range of virtual addresses are divided uniformly
    /// among CPUs. In the fast path, each CPU allocates from its own range.
    ///
    /// Lock order: must be acquired in ascending CPU ID order.
    local_allocators: CpuLocalBox<SpinLock<RangeAllocator>>,
    /// To improve deallocation scalability, deallocated ranges are added to
    /// the local CPU's allocator, even if the range was allocated from another
    /// CPU's range.
    ///
    /// After [`num_cpus()`] * [`DEALLOC_BATCH_SIZE`] deallocations,
    /// we will return the ranges that were deallocated on other CPUs back to
    /// their original CPU allocators.
    dealloc_counter: CpuLocalBox<AtomicUsize>,
}

const DEALLOC_BATCH_SIZE: usize = 8;
const FULL_RANGE: Range<Vaddr> = VMAR_LOWEST_ADDR..VMAR_CAP_ADDR;

impl PerCpuAllocator {
    /// Creates a new per-CPU allocator.
    pub fn new() -> Result<Self> {
        let local_allocators = alloc_cpu_local(|cpu| {
            let full_range = Self::full_range_on_cpu(cpu);
            SpinLock::new(RangeAllocator::new(full_range))
        })?;
        let dealloc_counter =
            alloc_cpu_local(|cpu| AtomicUsize::new(cpu.as_usize() * DEALLOC_BATCH_SIZE))?;

        Ok(Self {
            local_allocators,
            dealloc_counter,
        })
    }

    /// Allocates a virtual memory range of the given size and alignment.
    pub fn alloc(&self, size: usize, align: usize) -> Result<Vaddr> {
        let preempt_guard = disable_preempt();
        let cur_cpu = preempt_guard.current_cpu();
        let per_cpu_range_size = Self::full_range_on_cpu(CpuId::bsp()).len();

        match self.try_alloc_on_cpu(cur_cpu, size, align) {
            Ok(addr) => return Ok(addr),
            Err(err) if err.error() != Errno::ENOMEM => return Err(err),
            Err(_) => {}
        }

        if size <= per_cpu_range_size {
            for cpu in all_cpus() {
                if cpu == cur_cpu {
                    continue;
                }
                match self.try_alloc_on_cpu(cpu, size, align) {
                    Ok(addr) => return Ok(addr),
                    Err(err) if err.error() == Errno::ENOMEM => continue,
                    Err(err) => return Err(err),
                }
            }
        }

        // There's still a chance to allocate by merging free ranges from all CPUs.
        self.alloc_across_cpus(all_cpus(), AllocOption::General { size, align })
    }

    /// Allocates a specific virtual memory range.
    pub fn alloc_specific(&self, range: Range<Vaddr>) -> Result<Vaddr> {
        let cpuid_range = Self::cpu_id_range_that_covers(range.clone());

        if cpuid_range.len() == 1 {
            let cpu = CpuId::try_from(cpuid_range.start).unwrap();
            let mut allocator = self.local_allocators.get_on_cpu(cpu).lock();
            return allocator.alloc_specific(range);
        }

        self.alloc_across_cpus(
            cpuid_range.map(|id| CpuId::try_from(id).unwrap()),
            AllocOption::Specific(range),
        )
    }

    /// Deallocates a virtual memory range.
    ///
    /// # Panics
    ///
    /// Panics if the given range is already free.
    ///
    /// Note that due to batching, the deallocation may not immediately panic.
    pub fn dealloc(&self, range: Range<Vaddr>) {
        let preempt_guard = disable_preempt();
        let cur_cpu = preempt_guard.current_cpu();

        let mut allocator = self.local_allocators.get_on_cpu(cur_cpu).lock();
        allocator.add_range_try_merge(range);

        let counter = self.dealloc_counter.get_on_cpu(cur_cpu);
        let prev_count = counter.fetch_add(1, Ordering::Relaxed);
        let modulos = DEALLOC_BATCH_SIZE * num_cpus();

        if (prev_count + 1) % modulos != 0 {
            return;
        }

        counter.store(0, Ordering::Relaxed);

        self.return_ranges_to_other_cpus(cur_cpu, allocator);
    }

    fn full_range_on_cpu(cpu: CpuId) -> Range<Vaddr> {
        let per_cpu_range_size = FULL_RANGE.len() / num_cpus();
        let start = VMAR_LOWEST_ADDR + per_cpu_range_size * (cpu.as_usize());
        let end = start + per_cpu_range_size;
        start..end
    }

    fn cpu_id_range_that_covers(range: Range<Vaddr>) -> Range<usize> {
        let per_cpu_range_size = FULL_RANGE.len() / num_cpus();
        (range.start - VMAR_LOWEST_ADDR) / per_cpu_range_size
            ..(range.end - VMAR_LOWEST_ADDR - 1) / per_cpu_range_size + 1
    }

    fn try_alloc_on_cpu(&self, cpu: CpuId, size: usize, align: usize) -> Result<Vaddr> {
        let mut allocator = self.local_allocators.get_on_cpu(cpu).lock();
        allocator.alloc(size, align)
    }

    /// The CPU iter must be in the lock order.
    fn alloc_across_cpus(
        &self,
        cpus: impl ExactSizeIterator<Item = CpuId> + Clone,
        opt: AllocOption,
    ) -> Result<Vaddr> {
        if cpus.len() == 1 {
            return_errno_with_message!(Errno::ENOMEM, "out of virtual addresses");
        }

        let mut guards = Vec::with_capacity(cpus.len());
        for cpu in cpus.clone() {
            guards.push(self.local_allocators.get_on_cpu(cpu).lock());
        }

        // Merge all free ranges into a temporary full-range allocator.
        let mut full_allocator = RangeAllocator::new_empty();

        for allocator in &mut guards {
            for free in allocator.freelist.take(&FULL_RANGE) {
                full_allocator.add_range_try_merge(free.range());
            }
        }

        let res = match opt {
            AllocOption::Specific(range) => full_allocator.alloc_specific(range),
            AllocOption::General { size, align } => full_allocator.alloc(size, align),
        };

        // Return unallocated ranges back to per-CPU allocators.
        for (allocator, cpu) in guards.iter_mut().zip(cpus) {
            let full_range = Self::full_range_on_cpu(cpu);
            let mut remnant = Vec::new();
            for free in full_allocator.freelist.take(&full_range) {
                let (before, overlapping, after) = free.truncate(full_range.clone());

                if let Some(before) = before {
                    remnant.push(before);
                }
                if let Some(overlapping) = overlapping {
                    allocator.freelist.insert(overlapping);
                }
                if let Some(after) = after {
                    remnant.push(after);
                }
            }
            remnant
                .into_iter()
                .for_each(|r| allocator.freelist.insert(r));
        }

        res
    }

    fn return_ranges_to_other_cpus(
        &self,
        cur_cpu: CpuId,
        mut allocator: SpinLockGuard<'_, RangeAllocator, PreemptDisabled>,
    ) {
        if num_cpus() == 1 {
            return;
        }

        let mut ranges_from_others = Vec::new();
        for other in all_cpus() {
            if other == cur_cpu {
                continue;
            }
            let other_range = Self::full_range_on_cpu(other);
            let mut remnant = Vec::new();
            for r in allocator.freelist.take(&other_range) {
                let (before, overlapping, after) = r.truncate(other_range.clone());
                if let Some(before) = before {
                    remnant.push(before);
                }
                if let Some(overlapping) = overlapping {
                    ranges_from_others.push((other, overlapping));
                }
                if let Some(after) = after {
                    remnant.push(after);
                }
            }
            remnant
                .into_iter()
                .for_each(|r| allocator.freelist.insert(r));
        }

        // We need to drop the current lock guard before acquiring other CPU locks.
        // Because the locks must be acquired in ascending CPU ID order.
        drop(allocator);

        let mut cur_guard: Option<(CpuId, SpinLockGuard<'_, RangeAllocator, PreemptDisabled>)> =
            None;
        for (cpu, range) in ranges_from_others {
            if let Some((guard_cpu, guard)) = &mut cur_guard {
                if *guard_cpu == cpu {
                    guard.freelist.insert(range);
                    continue;
                }
            }
            debug_assert_ne!(cpu, cur_cpu);
            let mut guard = self.local_allocators.get_on_cpu(cpu).lock();
            guard.freelist.insert(range);
            cur_guard = Some((cpu, guard));
        }
    }
}

enum AllocOption {
    Specific(Range<Vaddr>),
    General { size: usize, align: usize },
}

struct RangeAllocator {
    freelist: IntervalSet<Vaddr, FreeRange>,
}

impl RangeAllocator {
    fn new_empty() -> Self {
        Self {
            freelist: IntervalSet::new(),
        }
    }

    fn new(full_range: Range<Vaddr>) -> Self {
        let mut freelist = IntervalSet::new();
        freelist.insert(FreeRange::new(full_range));
        Self { freelist }
    }

    fn alloc(&mut self, size: usize, align: usize) -> Result<Vaddr> {
        if size == 0 {
            return_errno_with_message!(Errno::EINVAL, "cannot allocate zero-sized range");
        }
        if align == 0 || !align.is_power_of_two() {
            return_errno_with_message!(Errno::EINVAL, "alignment must be a power of two");
        }

        let mut chosen_range = None;
        // Iterate in reverse and allocate the lowest address in the found
        // range to trade fragmentation for allocation speed.
        for free in self.freelist.iter().rev() {
            let range = free.range();
            let aligned_start = range.start.align_up(align);
            if aligned_start >= range.end {
                continue;
            }
            let alloc_end = aligned_start
                .checked_add(size)
                .ok_or_else(|| Error::with_message(Errno::ENOMEM, "address overflow"))?;
            if alloc_end > range.end {
                continue;
            }
            chosen_range = Some((range.start, aligned_start, alloc_end, range.end));
            break;
        }

        let Some((free_start, alloc_start, alloc_end, free_end)) = chosen_range else {
            return_errno_with_message!(Errno::ENOMEM, "out of virtual addresses");
        };

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

    fn alloc_specific(&mut self, range: Range<Vaddr>) -> Result<Vaddr> {
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
    fn add_range_try_merge(&mut self, range: Range<Vaddr>) {
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
}

struct FreeRange {
    block: Range<Vaddr>,
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
    fn truncate(self, range: Range<Vaddr>) -> (Option<Self>, Option<Self>, Option<Self>) {
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
    fn test_range_overlaps() {
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
    fn range_allocator_alloc_succeeds() {
        let mut allocator = RangeAllocator::new(0..128);

        let first = allocator.alloc(16, 8).expect("initial alloc fails");
        assert!(first % 8 == 0);

        let second = allocator
            .alloc(16, 32)
            .expect("aligned allocation should succeed");
        assert!(!range_overlaps(
            &(first..first + 16),
            &(second..second + 16)
        ));
        assert!(second % 32 == 0);

        let third = allocator.alloc(8, 16).expect("third alloc fails");
        assert!(!range_overlaps(&(first..first + 16), &(third..third + 8)));
        assert!(!range_overlaps(&(second..second + 16), &(third..third + 8)));
        assert!(third % 16 == 0);

        let free_ranges_total_size: usize = allocator
            .freelist
            .iter()
            .map(|free| free.range().len())
            .sum();
        assert_eq!(free_ranges_total_size, 128 - (16 + 16 + 8));
    }

    #[ktest]
    fn range_allocator_alloc_fails() {
        let mut allocator = RangeAllocator::new(0..64);

        let err = allocator.alloc(0, 8).unwrap_err();
        assert_eq!(err.error(), Errno::EINVAL);

        let err = allocator.alloc(8, 3).unwrap_err();
        assert_eq!(err.error(), Errno::EINVAL);

        allocator.alloc(48, 8).expect("able to consume most space");
        let err = allocator.alloc(32, 8).unwrap_err();
        assert_eq!(err.error(), Errno::ENOMEM);
    }

    #[ktest]
    fn range_allocator_alloc_specific_succeeds() {
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
    fn range_allocator_alloc_specific_fails() {
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
    fn range_allocator_add_will_merge() {
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
    fn range_allocator_add_duplicate() {
        let mut allocator = RangeAllocator::new_empty();
        allocator.add_range_try_merge(0..32);
        allocator.add_range_try_merge(16..48);
    }

    #[ktest]
    fn per_cpu_allocator_alloc_succeed() {
        let allocator = PerCpuAllocator::new().expect("per-CPU allocator must initialize");
        let size = PAGE_SIZE;
        let align = PAGE_SIZE;
        let addr = allocator.alloc(size, align).expect("per-CPU alloc failed");
        assert_eq!(addr % align, 0);

        let guard = disable_preempt();
        let cur_cpu = guard.current_cpu();
        drop(guard);

        let cpu_range = PerCpuAllocator::full_range_on_cpu(cur_cpu);
        let alloc_end = addr.checked_add(size).unwrap();
        assert!(addr >= cpu_range.start && alloc_end <= cpu_range.end);
    }

    #[ktest]
    fn per_cpu_allocator_alloc_fails() {
        let allocator = PerCpuAllocator::new().expect("per-CPU allocator must initialize");

        let err = allocator.alloc(0, PAGE_SIZE).unwrap_err();
        assert_eq!(err.error(), Errno::EINVAL);

        let err = allocator.alloc(PAGE_SIZE, 3).unwrap_err();
        assert_eq!(err.error(), Errno::EINVAL);
    }

    #[ktest]
    fn per_cpu_allocator_alloc_large_from_others() {
        let allocator = PerCpuAllocator::new().expect("per-CPU allocator must initialize");
        let per_cpu_range = PerCpuAllocator::full_range_on_cpu(CpuId::bsp());
        let size = per_cpu_range.len() + PAGE_SIZE;
        let result = allocator.alloc(size, PAGE_SIZE);

        if num_cpus() == 1 {
            assert_eq!(result.unwrap_err().error(), Errno::ENOMEM);
            return;
        }

        let addr = result.expect("should succeed with multiple CPUs");
        assert_eq!(addr, VMAR_LOWEST_ADDR);
        let end = addr.checked_add(size).unwrap();
        assert!(end <= VMAR_CAP_ADDR);
    }

    #[ktest]
    fn per_cpu_allocator_alloc_specific_succeed() {
        let allocator = PerCpuAllocator::new().expect("per-CPU allocator must initialize");
        let cpu = CpuId::bsp();
        let cpu_range = PerCpuAllocator::full_range_on_cpu(cpu);
        let target = cpu_range.start + PAGE_SIZE..cpu_range.start + 2 * PAGE_SIZE;

        allocator
            .alloc_specific(target.clone())
            .expect("specific allocation failed");

        let guard = allocator.local_allocators.get_on_cpu(cpu).lock();
        assert!(guard.freelist.find_one(&target.start).is_none());
    }

    #[ktest]
    fn per_cpu_allocator_alloc_specific_fails() {
        let allocator = PerCpuAllocator::new().expect("per-CPU allocator must initialize");
        let cpu = CpuId::bsp();
        let cpu_range = PerCpuAllocator::full_range_on_cpu(cpu);
        let target = cpu_range.start..(cpu_range.start + PAGE_SIZE);

        allocator
            .alloc_specific(target.clone())
            .expect("first specific allocation succeeds");
        let err = allocator.alloc_specific(target).unwrap_err();
        assert_eq!(err.error(), Errno::ENOMEM);
    }

    #[ktest]
    fn per_cpu_allocator_dealloc_realloc() {
        let allocator = PerCpuAllocator::new().expect("per-CPU allocator must initialize");
        let size = PAGE_SIZE;
        let addr = allocator.alloc(size, PAGE_SIZE).expect("allocation failed");
        let end = addr.checked_add(size).unwrap();
        allocator.dealloc(addr..end);

        let addr2 = allocator
            .alloc(size, PAGE_SIZE)
            .expect("re-allocation failed");
        assert_eq!(addr2, addr);
    }
}
