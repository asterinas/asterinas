// SPDX-License-Identifier: MPL-2.0

//! Utility functions to allocate and lock an unmapped range.

use alloc::vec::Vec;
use core::ops::Range;

use align_ext::AlignExt;
use osdk_heap_allocator::{CpuLocalBox, alloc_cpu_local};
use ostd::{
    cpu::{CpuId, PinCurrentCpu, all_cpus, num_cpus},
    mm::{HIGHEST_PAGING_LEVEL, PAGE_SIZE, Vaddr, num_subpages_per_huge, page_size_at},
    sync::{PreemptDisabled, SpinLock, SpinLockGuard},
    task::DisabledPreemptGuard,
    util::id_set::Id,
};

use crate::{
    error::{Errno, Error},
    prelude::Result,
    vm::vmar::{
        VMAR_CAP_ADDR, VMAR_LOWEST_ADDR, VmarSpace,
        interval_set::Interval,
        util::{get_intersected_range, is_intersected},
        vmar_impls::{PteRangeMeta, VmarCursorMut},
    },
};

/// A virtual address range allocator.
pub struct VirtualAddressAllocator {
    /// For each CPU, we maintain a range that must be free for fast allocation.
    must_be_free: CpuLocalBox<SpinLock<Range<Vaddr>>>,
}

/// A guard over the allocator to avoid races.
#[derive(Debug)]
pub struct AllocatorGuard<'a>(GuardInner<'a>);

#[derive(Debug)]
enum GuardInner<'a> {
    Single(SpinLockGuard<'a, Range<Vaddr>, PreemptDisabled>),
    Multi(Vec<SpinLockGuard<'a, Range<Vaddr>, PreemptDisabled>>),
}

impl VirtualAddressAllocator {
    /// Creates a new allocator that covers the entire VM range.
    pub fn new() -> Result<Self> {
        let must_be_free = alloc_cpu_local(|cpu| SpinLock::new(assigned_range_on_cpu(cpu)))?;
        Ok(Self { must_be_free })
    }

    /// Forks from another allocator and returns the copy.
    ///
    /// This must be called when the entire page table is locked.
    pub fn fork_from(&self) -> Result<Self> {
        let must_be_free =
            alloc_cpu_local(|cpu| SpinLock::new(self.must_be_free.get_on_cpu(cpu).lock().clone()))?;
        Ok(Self { must_be_free })
    }

    /// Marks a range as not-free and returns a cursor covering the range.
    pub fn alloc_specific_and_lock<'a>(
        &self,
        guard: &'a DisabledPreemptGuard,
        vmar_space: &'a VmarSpace,
        range: &Range<Vaddr>,
    ) -> (AllocatorGuard<'_>, VmarCursorMut<'a>) {
        self.alloc_specific_and_lock_larger(guard, vmar_space, range, range)
    }

    /// Marks a range as not-free and returns a cursor covering a larger range.
    pub fn alloc_specific_and_lock_larger<'s, 'a>(
        &'s self,
        guard: &'a DisabledPreemptGuard,
        vmar_space: &'a VmarSpace,
        range: &Range<Vaddr>,
        lock_range: &Range<Vaddr>,
    ) -> (AllocatorGuard<'s>, VmarCursorMut<'a>) {
        let cpu_range = cpu_range_for_vaddr_range(range);
        let guard_inner = if cpu_range.len() == 1 {
            let mut must_be_free = self
                .must_be_free
                .get_on_cpu(cpu_range.start.try_into().unwrap())
                .lock();
            *must_be_free = truncate_get_largest(&must_be_free, range);
            GuardInner::Single(must_be_free)
        } else {
            let mut vec = Vec::with_capacity(cpu_range.len());
            for cpu in cpu_range {
                let cpu_id = CpuId::try_from(cpu).unwrap();
                let mut must_be_free = self.must_be_free.get_on_cpu(cpu_id).lock();
                *must_be_free = truncate_get_largest(&must_be_free, range);
                vec.push(must_be_free);
            }
            GuardInner::Multi(vec)
        };

        let cursor = vmar_space.cursor_mut(guard, lock_range).unwrap();

        (AllocatorGuard(guard_inner), cursor)
    }

    /// Allocates a virtual address range and gets a cursor covering it.
    pub fn alloc_and_lock<'a>(
        &self,
        guard: &'a DisabledPreemptGuard,
        vmspace: &'a VmarSpace,
        size: usize,
        align: usize,
    ) -> Result<(Vaddr, AllocatorGuard<'_>, VmarCursorMut<'a>)> {
        let cur_cpu = guard.current_cpu();

        // Fast path by querying the `must_be_free` range.
        if let Ok((va, alloc_guard)) = self.alloc_from_cpu_must_free(cur_cpu, size, align) {
            let cursor = vmspace.cursor_mut(guard, &(va..va + size))?;
            return Ok((va, alloc_guard, cursor));
        }

        // Try other CPUs' fast path.
        for cpu_id in all_cpus() {
            if cpu_id == cur_cpu {
                continue;
            }
            if let Ok((va, alloc_guard)) = self.alloc_from_cpu_must_free(cpu_id, size, align) {
                let cursor = vmspace.cursor_mut(guard, &(va..va + size))?;
                return Ok((va, alloc_guard, cursor));
            }
        }

        // Try this CPUs' full assigned range.
        if let Ok(res) = self.try_lock_and_alloc_from(
            guard,
            vmspace,
            &assigned_range_on_cpu(cur_cpu),
            size,
            align,
        ) {
            return Ok(res);
        }

        // Try all the assigned ranges of each CPU.
        for cpu_id in all_cpus() {
            if cpu_id == cur_cpu {
                continue;
            }
            if let Ok(res) = self.try_lock_and_alloc_from(
                guard,
                vmspace,
                &assigned_range_on_cpu(cpu_id),
                size,
                align,
            ) {
                return Ok(res);
            }
        }

        // Try the full userspace range.
        let full_range = VMAR_LOWEST_ADDR..VMAR_CAP_ADDR;
        self.try_lock_and_alloc_from(guard, vmspace, &full_range, size, align)
    }

    /// Allocates a virtual address range and gets a cursor covering the given range.
    ///
    /// This function is similar to [`Self::alloc_and_lock`], but it also ensures that
    /// the returned cursor covers the given range.
    pub fn alloc_and_lock_covering_another<'a>(
        &self,
        guard: &'a DisabledPreemptGuard,
        vmspace: &'a VmarSpace,
        range: Range<Vaddr>,
        size: usize,
        align: usize,
    ) -> Result<(Vaddr, AllocatorGuard<'_>, VmarCursorMut<'a>)> {
        let cpu_range = cpu_range_for_vaddr_range(&range);

        // Fast path.
        if cpu_range.len() == 1
            && let Ok((va, alloc_guard)) = self.alloc_from_cpu_must_free(
                CpuId::try_from(cpu_range.start).unwrap(),
                size,
                align,
            )
        {
            let lock_range = range.start.min(va)..range.end.max(va + size);
            let cursor = vmspace.cursor_mut(guard, &lock_range)?;
            return Ok((va, alloc_guard, cursor));
        }

        // Slow path: allocate from the assigned ranges.
        let start_va = assigned_range_on_cpu(cpu_range.start.try_into().unwrap()).start;
        let end_va = assigned_range_on_cpu((cpu_range.end - 1).try_into().unwrap()).end;
        let range = start_va..end_va;

        if !range.is_empty()
            && let Ok(res) = self.try_lock_and_alloc_from(guard, vmspace, &range, size, align)
        {
            return Ok(res);
        }

        // Try the full userspace range.
        let full_range = VMAR_LOWEST_ADDR..VMAR_CAP_ADDR;
        self.try_lock_and_alloc_from(guard, vmspace, &full_range, size, align)
    }

    fn alloc_from_cpu_must_free(
        &self,
        cpu: CpuId,
        size: usize,
        align: usize,
    ) -> Result<(Vaddr, AllocatorGuard<'_>)> {
        let mut must_be_free = self.must_be_free.get_on_cpu(cpu).lock();

        alloc_from_free_range(must_be_free.clone(), size, align).map(|va| {
            *must_be_free = truncate_get_largest(&must_be_free, &(va..va + size));
            (va, AllocatorGuard(GuardInner::Single(must_be_free)))
        })
    }

    fn try_lock_and_alloc_from<'a>(
        &self,
        guard: &'a DisabledPreemptGuard,
        vmspace: &'a VmarSpace,
        range: &Range<Vaddr>,
        size: usize,
        align: usize,
    ) -> Result<(Vaddr, AllocatorGuard<'_>, VmarCursorMut<'a>)> {
        if !can_alloc_from_free_range(range, size, align) {
            return Err(Error::with_message(
                Errno::ENOMEM,
                "the given range cannot fit the allocation",
            ));
        }

        // Lock the relevant `must_be_free`s before the cursor otherwise
        // `remove_free_range` does not guarantee free of allocations from it.

        let cpu_range = cpu_range_for_vaddr_range(range);
        let mut alloc_guard = AllocatorGuard(if cpu_range.len() == 1 {
            GuardInner::Single(
                self.must_be_free
                    .get_on_cpu(cpu_range.start.try_into().unwrap())
                    .lock(),
            )
        } else {
            let mut vec = Vec::with_capacity(cpu_range.len());
            for cpu in cpu_range.clone() {
                vec.push(self.must_be_free.get_on_cpu(cpu.try_into().unwrap()).lock());
            }
            GuardInner::Multi(vec)
        });

        let mut cursor = vmspace.cursor_mut(guard, range)?;
        let (va, range) = find_fit_range_in_cursor(&mut cursor, size, align)?;

        // Maintain the must be free ranges.
        let remain = truncate_get_largest(&range, &(va..va + size));
        if !remain.is_empty() {
            match &mut alloc_guard {
                AllocatorGuard(GuardInner::Single(must_be_empty)) => {
                    if remain.len() > must_be_empty.len() {
                        **must_be_empty = remain;
                    }
                }
                AllocatorGuard(GuardInner::Multi(vec)) => {
                    for (cpu, must_be_empty) in cpu_range.zip(vec.iter_mut()) {
                        let cpu_id = CpuId::try_from(cpu).unwrap();
                        let assigned_range = assigned_range_on_cpu(cpu_id);
                        if is_intersected(&remain, &assigned_range) {
                            let free_remain = get_intersected_range(&remain, &assigned_range);
                            if free_remain.len() > must_be_empty.len() {
                                **must_be_empty = free_remain;
                            }
                        }
                    }
                }
            }
        }

        Ok((va, alloc_guard, cursor))
    }
}

/// Tries to allocate a virtual address range in the cursor's guard range.
///
/// It also returns the free range found in the process. The allocated address
/// is within the free range.
fn find_fit_range_in_cursor(
    cursor: &mut VmarCursorMut,
    size: usize,
    align: usize,
) -> Result<(Vaddr, Range<Vaddr>)> {
    let guard_range = cursor.guard_va_range();
    cursor.jump(guard_range.start).unwrap();

    // Search top-down for a suitable range.
    let mut search_end = guard_range.end;
    cursor.jump(search_end - PAGE_SIZE).unwrap();
    while (search_end - size).align_down(align) >= guard_range.start {
        let cur_level = cursor.level();
        let cur_va = cursor.virt_addr();

        let Some(meta) = cursor.aux_meta().inner.find_prev(&(cur_va + 1)) else {
            let cur_free_start = cursor.virt_addr().align_down(page_size_at(cur_level + 1));
            if let Ok(va) = alloc_from_free_range(cur_free_start..search_end, size, align) {
                return Ok((va, cur_free_start..search_end));
            }

            // Pop level and continue searching.
            if cursor.level() == cursor.guard_level() {
                break;
            } else {
                cursor.pop_level();
                let prev_addr = cursor.cur_va_range().start - PAGE_SIZE;
                if cursor.jump(prev_addr).is_err() {
                    break;
                }
                continue;
            }
        };
        let possible_end = meta.range().end.max(guard_range.start);
        if possible_end < search_end
            && let Ok(va) = alloc_from_free_range(possible_end..search_end, size, align)
        {
            return Ok((va, possible_end..search_end));
        }
        match meta {
            PteRangeMeta::ChildPt(r) => {
                debug_assert!(r.start < cursor.virt_addr());
                if r.end < guard_range.start {
                    break;
                }
                if r.end - PAGE_SIZE < cursor.virt_addr() {
                    cursor.jump(r.end - PAGE_SIZE).unwrap();
                }
                cursor.push_level_if_exists().unwrap();
            }
            PteRangeMeta::VmMapping(vm_mapping) => {
                if vm_mapping.range().start <= guard_range.start {
                    break;
                }
                search_end = vm_mapping.range().start;
                if cursor.jump(search_end - PAGE_SIZE).is_err() {
                    break;
                }
            }
        }
    }

    Err(Error::with_message(
        Errno::ENOMEM,
        "no suitable range found",
    ))
}

/// Returns the assigned virtual address range for the given CPU.
///
/// The assigned range is firstly searched for free space to allocate.
fn assigned_range_on_cpu(cpu: CpuId) -> Range<Vaddr> {
    let num_cpus = num_cpus();
    let per_cpu_range_size = assigned_range_size_for(num_cpus);

    let start = per_cpu_range_size * (cpu.as_usize());
    let end = if cpu.as_usize() == num_cpus - 1 {
        // Add the residue to the last CPU's range.
        VMAR_CAP_ADDR
    } else {
        start + per_cpu_range_size
    };

    start.max(VMAR_LOWEST_ADDR)..end.max(VMAR_LOWEST_ADDR)
}

/// Returns the range of CPU IDs that the given virtual address belongs to.
fn cpu_range_for_vaddr_range(range: &Range<Vaddr>) -> Range<usize> {
    debug_assert!(range.start < range.end);

    let num_cpus = num_cpus();
    let per_cpu_range_size = assigned_range_size_for(num_cpus);

    let start_cpu = (range.start / per_cpu_range_size).min(num_cpus - 1);
    let end_cpu = ((range.end - 1) / per_cpu_range_size).min(num_cpus - 1);

    start_cpu..end_cpu + 1
}

/// Returns the assigned range size for CPUs (except for the last CPU).
///
/// Try out best to make the size aligned so the lock protocol performs well.
const fn assigned_range_size_for(num_cpus: usize) -> usize {
    let align_to_paging_level = if num_cpus < num_subpages_per_huge() / 2 {
        HIGHEST_PAGING_LEVEL
    } else if num_cpus < num_subpages_per_huge().pow(2) / 2 {
        HIGHEST_PAGING_LEVEL - 1
    } else if num_cpus < num_subpages_per_huge().pow(3) / 2 {
        HIGHEST_PAGING_LEVEL - 2
    } else {
        HIGHEST_PAGING_LEVEL - 3
    };

    let align_to = page_size_at(align_to_paging_level);

    (VMAR_CAP_ADDR / num_cpus) & !(align_to - 1)
}

/// Allocates a virtual address range from the given free range.
fn alloc_from_free_range(range: Range<Vaddr>, size: usize, align: usize) -> Result<Vaddr> {
    if range.len() > size {
        let aligned_start = (range.end - size).align_down(align);
        if aligned_start >= range.start {
            return Ok(aligned_start);
        }
    }

    Err(Error::with_message(
        Errno::ENOMEM,
        "no suitable range found in the free range",
    ))
}

fn can_alloc_from_free_range(range: &Range<Vaddr>, size: usize, align: usize) -> bool {
    alloc_from_free_range(range.clone(), size, align).is_ok()
}

/// Calculates `range` - `cut` and return the largest remaining piece.
fn truncate_get_largest(range: &Range<Vaddr>, cut: &Range<Vaddr>) -> Range<Vaddr> {
    if !is_intersected(range, cut) {
        return range.clone();
    }

    let left = range.start..cut.start.max(range.start);
    let right = cut.end.min(range.end)..range.end;

    if left.len() > right.len() {
        left
    } else {
        right
    }
}
