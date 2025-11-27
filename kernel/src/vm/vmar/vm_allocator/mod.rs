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
use range_allocator::{AllocOption, RangeAllocator};

use super::{Interval, VMAR_CAP_ADDR, VMAR_LOWEST_ADDR};
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

    /// Replaces the state from another allocator.
    ///
    /// Note that this function does not return a well-formed allocator if
    /// called from multiple CPUs. The caller must synchronize appropriately.
    pub fn fork_from(&mut self, other: &Self) {
        for cpu in all_cpus() {
            let other_allocator = other.local_allocators.get_on_cpu(cpu).lock();
            let mut self_allocator = self.local_allocators.get_on_cpu(cpu).lock();
            self_allocator.fork_from(&other_allocator);
        }
    }

    /// Allocates a virtual memory range of the given size and alignment.
    ///
    /// The allocation algorithm does not have any preference on the address
    /// of the allocated range (e.g., near top or near bottom). However,
    /// this is the most scalable allocation algorithm.
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

    /// Allocates a range from the top of the virtual address space.
    ///
    /// Unlike [`Self::alloc`], this method returns the highest possible range
    /// that fits the requested size and alignment. But this is not scalable.
    pub fn alloc_top(&self, size: usize, align: usize) -> Result<Vaddr> {
        let last_cpu = CpuId::try_from(num_cpus() - 1).unwrap();

        if Self::full_range_on_cpu(last_cpu).len() >= size {
            let mut allocator = self.local_allocators.get_on_cpu(last_cpu).lock();
            if let Ok(res) = allocator.alloc(size, align, RangeAllocator::find_top_slow) {
                return Ok(res);
            }
        }

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

        if !(prev_count + 1).is_multiple_of(modulos) {
            return;
        }

        counter.store(0, Ordering::Relaxed);

        self.return_ranges_to_other_cpus(cur_cpu, allocator);
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
        (range.start - VMAR_LOWEST_ADDR) / per_cpu_range_size
            ..(range.end - VMAR_LOWEST_ADDR - 1) / per_cpu_range_size + 1
    }

    fn try_alloc_on_cpu(&self, cpu: CpuId, size: usize, align: usize) -> Result<Vaddr> {
        let mut allocator = self.local_allocators.get_on_cpu(cpu).lock();
        allocator.alloc(size, align, RangeAllocator::find_top_fast)
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
            for free in allocator.take_free_ranges(&FULL_RANGE) {
                full_allocator.add_range_try_merge(free.range());
            }
        }

        let res = match opt {
            AllocOption::Specific(range) => full_allocator.alloc_specific(range),
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
                .for_each(|r| allocator.add_range_without_merge(r));
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
        drop(allocator);

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
            let mut guard = self.local_allocators.get_on_cpu(cpu).lock();
            guard.add_range_try_merge(range.range());
            cur_guard = Some((cpu, guard));
        }
    }
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

        let addr = result.expect("should succeed with multiple CPUs");
        assert_eq!(addr, VMAR_LOWEST_ADDR);
        let end = addr.checked_add(size).unwrap();
        assert!(end <= VMAR_CAP_ADDR);
    }

    #[ktest]
    fn alloc_top_is_topmost() {
        let allocator = PerCpuAllocator::new().expect("per-CPU allocator must initialize");

        let addr = allocator
            .alloc_top(PAGE_SIZE * 10, PAGE_SIZE)
            .expect("alloc_top failed");
        assert_eq!(addr, VMAR_CAP_ADDR - PAGE_SIZE * 10);

        let addr2 = allocator
            .alloc_top(PAGE_SIZE * 100, PAGE_SIZE * 512)
            .expect("second alloc_top failed");
        assert_eq!(
            addr2,
            (VMAR_CAP_ADDR - PAGE_SIZE * 100).align_down(PAGE_SIZE * 512)
        );

        let addr3 = allocator
            .alloc_top(PAGE_SIZE, PAGE_SIZE)
            .expect("third alloc_top failed");
        assert_eq!(addr3, addr - PAGE_SIZE);
    }

    #[ktest]
    fn alloc_specific_succeed() {
        let allocator = PerCpuAllocator::new().expect("per-CPU allocator must initialize");
        let cpu = CpuId::bsp();
        let cpu_range = PerCpuAllocator::full_range_on_cpu(cpu);
        let target = cpu_range.start + PAGE_SIZE..cpu_range.start + 2 * PAGE_SIZE;

        allocator
            .alloc_specific(target.clone())
            .expect("specific allocation failed");

        let err = allocator.alloc_specific(target).unwrap_err();
        assert_eq!(err.error(), Errno::ENOMEM);
    }

    #[ktest]
    fn alloc_specific_fails() {
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
    fn dealloc_realloc() {
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

    #[ktest]
    fn fork_from_identical() {
        let allocator = PerCpuAllocator::new().expect("per-CPU allocator allocation fails");
        let _ = allocator
            .alloc(PAGE_SIZE * 10, PAGE_SIZE)
            .expect("initial alloc fails");
        let _ = allocator
            .alloc(PAGE_SIZE * 100, PAGE_SIZE * 8)
            .expect("second alloc fails");

        let mut forked = PerCpuAllocator::new().expect("per-CPU allocator allocation fails");
        forked.fork_from(&allocator);

        all_cpus().for_each(|cpu| {
            let original_guard = allocator.local_allocators.get_on_cpu(cpu).lock();
            let forked_guard = forked.local_allocators.get_on_cpu(cpu).lock();

            assert_eq!(*original_guard, *forked_guard);
        });
    }
}
