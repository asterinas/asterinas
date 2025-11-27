// SPDX-License-Identifier: MPL-2.0

//! Per-CPU allocator for virtual memory ranges.

mod range_allocator;
use alloc::vec::Vec;
use core::{
    ops::Range,
    sync::atomic::{AtomicUsize, Ordering},
};

use align_ext::AlignExt;
use osdk_heap_allocator::{CpuLocalBox, alloc_cpu_local};
use ostd::{
    cpu::{CpuId, PinCurrentCpu, all_cpus, num_cpus},
    mm::Vaddr,
    sync::PreemptDisabled,
    task::disable_preempt,
    util::id_set::Id,
};
use range_allocator::RangeAllocator;

use super::{VMAR_CAP_ADDR, VMAR_LOWEST_ADDR, interval_set::Interval};
use crate::prelude::*;

/// The per-CPU allocator for virtual memory addresses.
#[derive(Debug)]
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

    /// Replaces the state from another allocator.
    ///
    /// The caller must synchronize appropriately to ensure no concurrent
    /// allocations or deallocations are happening on the other allocator.
    pub fn fork_from(&mut self, other: &Self) {
        for cpu in all_cpus() {
            let other_allocator = other.local_allocators.get_on_cpu(cpu).lock();
            let mut self_allocator = self.local_allocators.get_on_cpu(cpu).lock();
            self_allocator.fork_from(&other_allocator);
        }
    }

    /// Resets the allocator to cover the full range.
    ///
    /// The caller must synchronize appropriately to ensure no concurrent
    /// allocations or deallocations are happening on this allocator.
    pub fn reset(&self) {
        for cpu in all_cpus() {
            let full_range = Self::full_range_on_cpu(cpu);
            let mut allocator = self.local_allocators.get_on_cpu(cpu).lock();
            allocator.reset(full_range);
        }
    }

    /// Locks a suitable per-cpu allocator for deallocation.
    ///
    /// The returned guard can be used to call [`PerCpuAllocatorGuard::dealloc`].
    pub fn lock_for_dealloc(&self) -> PerCpuAllocatorGuard<'_> {
        // `current_racy` is OK because locking any allocators is OK.
        let guard = self
            .local_allocators
            .get_on_cpu(CpuId::current_racy())
            .lock();
        PerCpuAllocatorGuard {
            inner: GuardInner::Single(guard),
            allocator: self,
            has_deallocated: false,
        }
    }

    /// Allocates a virtual memory range of the given size and alignment.
    ///
    /// The allocation algorithm does not have any preference on the address
    /// of the allocated range (e.g., near top or near bottom). However,
    /// this is the most scalable allocation algorithm.
    pub fn alloc(&self, size: usize, align: usize) -> Result<(PerCpuAllocatorGuard<'_>, Vaddr)> {
        let preempt_guard = disable_preempt();
        let cur_cpu = preempt_guard.current_cpu();
        let per_cpu_range_size = Self::full_range_on_cpu(CpuId::bsp()).len();

        match self.try_alloc_on_cpu(cur_cpu, size, align, true) {
            Ok((guard, res)) => return Ok((guard, res.start_addr)),
            Err(err) if err.error() != Errno::ENOMEM => return Err(err),
            Err(_) => {}
        }

        if size <= per_cpu_range_size {
            for cpu in all_cpus() {
                if cpu == cur_cpu {
                    continue;
                }
                match self.try_alloc_on_cpu(cpu, size, align, true) {
                    Ok((guard, res)) => return Ok((guard, res.start_addr)),
                    Err(err) if err.error() == Errno::ENOMEM => return Err(err),
                    Err(_) => continue,
                }
            }
        }

        // There's still a chance to allocate by merging free ranges from all CPUs.
        self.alloc_across_cpus(all_cpus(), AllocOption::General { size, align })
            .map(|(guard, res)| (guard, res.start_addr))
    }

    /// Allocates a range from the top of the virtual address space.
    ///
    /// Unlike [`Self::alloc`], this method returns the highest possible range
    /// that fits the requested size and alignment. But this is not scalable.
    pub fn alloc_top(
        &self,
        size: usize,
        align: usize,
    ) -> Result<(PerCpuAllocatorGuard<'_>, Vaddr)> {
        let last_cpu = CpuId::try_from(num_cpus() - 1).unwrap();

        if Self::full_range_on_cpu(last_cpu).len() >= size
            && let Ok((guard, res)) = self.try_alloc_on_cpu(last_cpu, size, align, false)
        {
            return Ok((guard, res.start_addr));
        }

        self.alloc_across_cpus(all_cpus(), AllocOption::General { size, align })
            .map(|(guard, res)| (guard, res.start_addr))
    }

    /// Allocates a specific virtual memory range.
    pub fn alloc_specific(&self, range: Range<Vaddr>) -> Result<PerCpuAllocatorGuard<'_>> {
        let cpuid_range = Self::cpu_id_range_that_covers(range.clone());

        if cpuid_range.len() == 1 {
            let cpu = CpuId::try_from(cpuid_range.start).unwrap();
            let mut allocator = self.local_allocators.get_on_cpu(cpu).lock();
            if allocator.alloc_specific(range.clone()).is_ok() {
                return Ok(PerCpuAllocatorGuard {
                    inner: GuardInner::Single(allocator),
                    allocator: self,
                    has_deallocated: false,
                });
            }
        } else if let Ok((guard, _)) = self.alloc_across_cpus(
            cpuid_range.map(|id| CpuId::try_from(id).unwrap()),
            AllocOption::Specific(range.clone()),
        ) {
            return Ok(guard);
        }

        // `alloc_specific` may spuriously fail because that the ranges
        // deallocated to other CPUs hasn't been returned yet. So we
        // fallthrough to `alloc_across_cpus` for any failure.
        self.alloc_across_cpus(all_cpus(), AllocOption::Specific(range))
            .map(|(guard, _)| guard)
    }

    /// Like [`Self::alloc_specific`], but overwrites any existing allocations.
    ///
    /// If there are ranges already allocated within the given range, this
    /// function will behave as if those ranges are added back with
    /// [`Self::dealloc`] first.
    ///
    /// The number of bytes overwritten is returned.
    pub fn alloc_specific_overwrite(
        &self,
        range: Range<Vaddr>,
    ) -> Result<(PerCpuAllocatorGuard<'_>, usize)> {
        let cpuid_range = Self::cpu_id_range_that_covers(range.clone());

        if cpuid_range.len() == 1 {
            let cpu = CpuId::try_from(cpuid_range.start).unwrap();
            let mut allocator = self.local_allocators.get_on_cpu(cpu).lock();
            if let Ok(overwritten_bytes) = allocator.alloc_specific_overwrite(range.clone()) {
                return Ok((
                    PerCpuAllocatorGuard {
                        inner: GuardInner::Single(allocator),
                        allocator: self,
                        has_deallocated: false,
                    },
                    overwritten_bytes,
                ));
            }
        } else if let Ok((guard, res)) = self.alloc_across_cpus(
            cpuid_range.map(|id| CpuId::try_from(id).unwrap()),
            AllocOption::SpecificOverwrite(range.clone()),
        ) {
            return Ok((guard, res.overwritten_bytes));
        }

        self.alloc_across_cpus(all_cpus(), AllocOption::SpecificOverwrite(range))
            .map(|(guard, res)| (guard, res.overwritten_bytes))
    }

    /// Counts the total free size in bytes that overlaps with the given range.
    ///
    /// It is inaccurate if there are concurrent allocations or deallocations.
    pub fn count_free_size_overlapping(&self, range: &Range<Vaddr>) -> usize {
        let mut total_free = 0;

        // Note that we can `dealloc` ranges to other CPU allocators. So
        // checking only `cpu_id_range_that_covers` is not enough.
        //
        // This function is only used for RLIMIT checking upon `mmap(FIXED)`
        // when the global RLIMIT checking fails. So we trade this corner-
        // case scalability for `munmap` scalability.
        for cpu in all_cpus() {
            let allocator = self.local_allocators.get_on_cpu(cpu).lock();
            total_free += allocator.count_free_size_overlapping(range);
        }

        total_free
    }

    fn full_range_on_cpu(cpu: CpuId) -> Range<Vaddr> {
        let num_cpus = num_cpus();
        let per_cpu_range_size = FULL_RANGE.len() / num_cpus;

        let start = VMAR_LOWEST_ADDR + per_cpu_range_size * (cpu.as_usize());
        let end = if cpu.as_usize() == num_cpus - 1 {
            // Add the residue to the last CPU's range.
            FULL_RANGE.end
        } else {
            start + per_cpu_range_size
        };

        start..end
    }

    fn cpu_id_range_that_covers(range: Range<Vaddr>) -> Range<usize> {
        let per_cpu_range_size = FULL_RANGE.len() / num_cpus();
        debug_assert!(per_cpu_range_size > 0);

        // Clamp to the last CPU to handle the residual range that the last
        // CPU owns (`FULL_RANGE` may not divide evenly by num_cpus).
        let start_off = range.start.saturating_sub(VMAR_LOWEST_ADDR);
        let end_off = range.end.saturating_sub(1).saturating_sub(VMAR_LOWEST_ADDR);

        let start = (start_off / per_cpu_range_size).min(num_cpus() - 1);
        let end = (end_off / per_cpu_range_size).min(num_cpus() - 1) + 1;

        start..end
    }

    fn try_alloc_on_cpu(
        &self,
        cpu: CpuId,
        size: usize,
        align: usize,
        fast: bool,
    ) -> Result<(PerCpuAllocatorGuard<'_>, AllocResult)> {
        let mut allocator = self.local_allocators.get_on_cpu(cpu).lock();
        let vaddr = allocator.alloc(
            size,
            align,
            if fast {
                RangeAllocator::find_top_fast
            } else {
                RangeAllocator::find_top_slow
            },
        )?;
        Ok((
            PerCpuAllocatorGuard {
                inner: GuardInner::Single(allocator),
                allocator: self,
                has_deallocated: false,
            },
            AllocResult {
                start_addr: vaddr,
                overwritten_bytes: 0,
            },
        ))
    }

    /// The CPU iter must be in the lock order.
    ///
    /// Returns the allocated address and the number of overwritten bytes.
    fn alloc_across_cpus(
        &self,
        cpus: impl ExactSizeIterator<Item = CpuId> + Clone,
        opt: AllocOption,
    ) -> Result<(PerCpuAllocatorGuard<'_>, AllocResult)> {
        debug_assert!(cpus.len() > 0);
        if cpus.len() == 1 {
            return_errno_with_message!(
                Errno::ENOMEM,
                "out of virtual addresses and no other CPUs to borrow from"
            );
        }

        let mut guards = Vec::with_capacity(cpus.len());
        for cpu in cpus.clone() {
            guards.push(self.local_allocators.get_on_cpu(cpu).lock());
        }

        // Merge all free ranges into a temporary full-range allocator.
        let mut full_allocator = RangeAllocator::new_empty();

        for allocator in &mut guards {
            for free in allocator.take_free_ranges(&FULL_RANGE) {
                full_allocator.add_range_try_merge(free.range());
            }
        }

        let mut overwritten_bytes = 0;

        let res = match opt {
            AllocOption::Specific(range) => full_allocator
                .alloc_specific(range.clone())
                .map(|_| range.start),
            AllocOption::SpecificOverwrite(range) => full_allocator
                .alloc_specific_overwrite(range.clone())
                .map(|b| {
                    overwritten_bytes = b;
                    range.start
                }),
            AllocOption::General { size, align } => {
                full_allocator.alloc(size, align, RangeAllocator::find_top_slow)
            }
        };

        // Return unallocated ranges back to per-CPU allocators.
        for (allocator, cpu) in guards.iter_mut().zip(cpus) {
            let full_range = Self::full_range_on_cpu(cpu);
            let mut remnant = Vec::new();
            for free in full_allocator.take_free_ranges(&full_range) {
                let (before, overlapping, after) = free.truncate(full_range.clone());

                if let Some(before) = before {
                    remnant.push(before);
                }
                if let Some(overlapping) = overlapping {
                    allocator.add_range_without_merge(overlapping);
                }
                if let Some(after) = after {
                    remnant.push(after);
                }
            }
            remnant
                .into_iter()
                .for_each(|r| full_allocator.add_range_without_merge(r));
        }

        let vaddr = res?;

        Ok((
            PerCpuAllocatorGuard {
                inner: GuardInner::Multiple(guards),
                allocator: self,
                has_deallocated: false,
            },
            AllocResult {
                start_addr: vaddr,
                overwritten_bytes,
            },
        ))
    }
}

/// The result of allocation.
#[derive(Debug)]
pub struct AllocResult {
    pub start_addr: Vaddr,
    pub overwritten_bytes: usize,
}

/// A guard that locks a range of virtual addresses.
///
/// When this guard is held, the recently allocated range will not be
/// overwritten by another thread.
#[derive(Debug)]
pub struct PerCpuAllocatorGuard<'a> {
    inner: GuardInner<'a>,
    allocator: &'a PerCpuAllocator,
    has_deallocated: bool,
}

#[derive(Debug)]
enum GuardInner<'a> {
    Single(SpinLockGuard<'a, RangeAllocator, PreemptDisabled>),
    Multiple(Vec<SpinLockGuard<'a, RangeAllocator, PreemptDisabled>>),
    /// This variant is only used to implement [`Drop`]. Not possible when
    /// the guard is alive.
    Dropping,
}

impl PerCpuAllocatorGuard<'_> {
    /// Deallocates a virtual memory range.
    ///
    /// # Panics
    ///
    /// Panics if the given range is already free.
    ///
    /// Note that due to batching, the deallocation may not immediately panic.
    pub fn dealloc(&mut self, range: Range<Vaddr>) {
        match &mut self.inner {
            GuardInner::Single(guard) => {
                guard.add_range_try_merge(range);
            }
            GuardInner::Multiple(guards) => {
                guards.first_mut().unwrap().add_range_try_merge(range);
            }
            GuardInner::Dropping => unreachable!(),
        }
        self.has_deallocated = true;
    }
}

impl Drop for PerCpuAllocatorGuard<'_> {
    fn drop(&mut self) {
        if num_cpus() == 1 || !self.has_deallocated {
            return;
        }

        // `current_racy` is OK because the lock guards exist.
        let cur_cpu = CpuId::current_racy();
        let counter = self.allocator.dealloc_counter.get_on_cpu(cur_cpu);
        let prev_count = counter.fetch_add(1, Ordering::Relaxed);
        let modulos = DEALLOC_BATCH_SIZE * num_cpus();

        if !(prev_count + 1).is_multiple_of(modulos) {
            return;
        }

        counter.store(0, Ordering::Relaxed);

        let allocator = match &mut self.inner {
            GuardInner::Single(guard) => guard,
            GuardInner::Multiple(guards) => guards.first_mut().unwrap(),
            GuardInner::Dropping => unreachable!(),
        };

        let mut ranges_from_others = Vec::new();
        for other in all_cpus() {
            if other == cur_cpu {
                continue;
            }
            let other_range = PerCpuAllocator::full_range_on_cpu(other);
            let mut remnant = Vec::new();
            for r in allocator.take_free_ranges(&other_range) {
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
                .for_each(|r| allocator.add_range_without_merge(r));
        }

        // We need to drop the current lock guard before acquiring other CPU locks.
        // Because the locks must be acquired in ascending CPU ID order.
        drop(core::mem::replace(&mut self.inner, GuardInner::Dropping));

        let mut cur_guard: Option<(CpuId, SpinLockGuard<'_, RangeAllocator, PreemptDisabled>)> =
            None;
        for (cpu, range) in ranges_from_others {
            if let Some((guard_cpu, guard)) = &mut cur_guard
                && *guard_cpu == cpu
            {
                guard.add_range_try_merge(range.range());
                continue;
            }

            debug_assert_ne!(cpu, cur_cpu);
            let mut guard = self.allocator.local_allocators.get_on_cpu(cpu).lock();
            guard.add_range_try_merge(range.range());
            cur_guard = Some((cpu, guard));
        }
    }
}

/// Options for allocation.
///
/// Used for [`PerCpuAllocator::alloc_across_cpus`].
#[derive(Debug)]
enum AllocOption {
    Specific(Range<Vaddr>),
    SpecificOverwrite(Range<Vaddr>),
    General { size: usize, align: usize },
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::*;

    use super::*;

    #[ktest]
    fn alloc_succeed() {
        let allocator = PerCpuAllocator::new().expect("per-CPU allocator must initialize");
        let size = PAGE_SIZE;
        let align = PAGE_SIZE;
        let (_, addr) = allocator.alloc(size, align).expect("per-CPU alloc failed");
        assert_eq!(addr % align, 0);

        let guard = disable_preempt();
        let cur_cpu = guard.current_cpu();
        drop(guard);

        let cpu_range = PerCpuAllocator::full_range_on_cpu(cur_cpu);
        let alloc_end = addr.checked_add(size).unwrap();
        assert!(addr >= cpu_range.start && alloc_end <= cpu_range.end);
    }

    #[ktest]
    fn alloc_fails() {
        let allocator = PerCpuAllocator::new().expect("per-CPU allocator must initialize");

        let err = allocator.alloc(0, PAGE_SIZE).unwrap_err();
        assert_eq!(err.error(), Errno::EINVAL);

        let err = allocator.alloc(PAGE_SIZE, 3).unwrap_err();
        assert_eq!(err.error(), Errno::EINVAL);
    }

    #[ktest]
    fn alloc_large_from_others() {
        let allocator = PerCpuAllocator::new().expect("per-CPU allocator must initialize");
        let per_cpu_range = PerCpuAllocator::full_range_on_cpu(CpuId::bsp());
        let size = per_cpu_range.len() + PAGE_SIZE;
        let result = allocator.alloc(size, PAGE_SIZE);

        if num_cpus() == 1 {
            assert_eq!(result.unwrap_err().error(), Errno::ENOMEM);
            return;
        }

        let addr = result.expect("should succeed with multiple CPUs").1;
        assert_eq!(addr, VMAR_LOWEST_ADDR);
        let end = addr.checked_add(size).unwrap();
        assert!(end <= VMAR_CAP_ADDR);
    }

    #[ktest]
    fn alloc_top_is_topmost() {
        let allocator = PerCpuAllocator::new().expect("per-CPU allocator must initialize");

        let addr = allocator
            .alloc_top(PAGE_SIZE * 10, PAGE_SIZE)
            .expect("alloc_top failed")
            .1;
        assert_eq!(addr, VMAR_CAP_ADDR - PAGE_SIZE * 10);

        let addr2 = allocator
            .alloc_top(PAGE_SIZE * 100, PAGE_SIZE * 512)
            .expect("second alloc_top failed")
            .1;
        assert_eq!(
            addr2,
            (VMAR_CAP_ADDR - PAGE_SIZE * 100).align_down(PAGE_SIZE * 512)
        );

        let addr3 = allocator
            .alloc_top(PAGE_SIZE, PAGE_SIZE)
            .expect("third alloc_top failed")
            .1;
        assert_eq!(addr3, addr - PAGE_SIZE);
    }

    #[ktest]
    fn alloc_specific_succeed() {
        let allocator = PerCpuAllocator::new().expect("per-CPU allocator must initialize");
        let cpu = CpuId::bsp();
        let cpu_range = PerCpuAllocator::full_range_on_cpu(cpu);
        let target = cpu_range.start + PAGE_SIZE..cpu_range.start + 2 * PAGE_SIZE;

        drop(
            allocator
                .alloc_specific(target.clone())
                .expect("specific allocation failed"),
        );

        let err = allocator.alloc_specific(target).unwrap_err();
        assert_eq!(err.error(), Errno::ENOMEM);
    }

    #[ktest]
    fn alloc_specific_fails() {
        let allocator = PerCpuAllocator::new().expect("per-CPU allocator must initialize");
        let cpu = CpuId::bsp();
        let cpu_range = PerCpuAllocator::full_range_on_cpu(cpu);
        let target = cpu_range.start..(cpu_range.start + PAGE_SIZE);

        drop(
            allocator
                .alloc_specific(target.clone())
                .expect("first specific allocation succeeds"),
        );
        let err = allocator.alloc_specific(target).unwrap_err();
        assert_eq!(err.error(), Errno::ENOMEM);
    }

    #[ktest]
    fn alloc_specific_overwrite() {
        let allocator = PerCpuAllocator::new().expect("per-CPU allocator must initialize");
        let cpu = CpuId::bsp();
        let cpu_range = PerCpuAllocator::full_range_on_cpu(cpu);
        let target = cpu_range.start + PAGE_SIZE..cpu_range.start + 4 * PAGE_SIZE;

        drop(
            allocator
                .alloc_specific(target.clone())
                .expect("first specific allocation succeeds"),
        );

        let overwritten = allocator
            .alloc_specific_overwrite(
                cpu_range.start + 2 * PAGE_SIZE..cpu_range.start + 5 * PAGE_SIZE,
            )
            .expect("specific overwrite allocation succeeds")
            .1;
        assert_eq!(overwritten, PAGE_SIZE * 2);

        let err = allocator.alloc_specific(target).unwrap_err();
        assert_eq!(err.error(), Errno::ENOMEM);
    }

    #[ktest]
    fn dealloc_realloc() {
        let allocator = PerCpuAllocator::new().expect("per-CPU allocator must initialize");
        let size = PAGE_SIZE;
        let (mut guard, addr) = allocator.alloc(size, PAGE_SIZE).expect("allocation failed");
        let end = addr.checked_add(size).unwrap();

        guard.dealloc(addr..end);
        drop(guard);

        let addr2 = allocator
            .alloc(size, PAGE_SIZE)
            .expect("re-allocation failed")
            .1;
        assert_eq!(addr2, addr);
    }

    #[ktest]
    fn fork_from_identical() {
        let allocator = PerCpuAllocator::new().expect("per-CPU allocator allocation fails");
        drop(
            allocator
                .alloc(PAGE_SIZE * 10, PAGE_SIZE)
                .expect("initial alloc fails"),
        );
        drop(
            allocator
                .alloc(PAGE_SIZE * 100, PAGE_SIZE * 8)
                .expect("second alloc fails"),
        );

        let mut forked = PerCpuAllocator::new().expect("per-CPU allocator allocation fails");
        forked.fork_from(&allocator);

        all_cpus().for_each(|cpu| {
            let original_guard = allocator.local_allocators.get_on_cpu(cpu).lock();
            let forked_guard = forked.local_allocators.get_on_cpu(cpu).lock();

            assert_eq!(*original_guard, *forked_guard);
        });
    }

    #[ktest]
    fn count_free_size_overlapping() {
        let allocator = PerCpuAllocator::new().expect("per-CPU allocator allocation fails");
        let cpu = CpuId::bsp();
        let cpu_range = PerCpuAllocator::full_range_on_cpu(cpu);

        let total_free = allocator.count_free_size_overlapping(&cpu_range);
        assert_eq!(total_free, cpu_range.len());

        drop(
            allocator
                .alloc(PAGE_SIZE * 10, PAGE_SIZE)
                .expect("allocation fails"),
        );

        let total_free = allocator.count_free_size_overlapping(&cpu_range);
        assert_eq!(total_free, cpu_range.len() - PAGE_SIZE * 10);
    }
}
