// SPDX-License-Identifier: MPL-2.0

//! Utility functions to allocate and lock an unmapped range.

use core::ops::{Range, RangeInclusive};

use align_ext::AlignExt;
use ostd::{
    cpu::{CpuId, PinCurrentCpu, all_cpus, num_cpus},
    mm::{HIGHEST_PAGING_LEVEL, PAGE_SIZE, Vaddr, num_subpages_per_huge, page_size_at},
    task::DisabledPreemptGuard,
    util::id_set::Id,
};

use crate::{
    error::{Errno, Error},
    prelude::Result,
    vm::vmar::{
        VMAR_CAP_ADDR, VMAR_LOWEST_ADDR, VmarSpace,
        cursor_util::find_next_mapped,
        interval_set::Interval,
        vmar_impls::{PteRangeMeta, VmarCursorMut},
    },
};

/// Allocates a virtual address range and gets a cursor covering it.
pub fn lock_and_alloc<'a>(
    guard: &'a DisabledPreemptGuard,
    vmspace: &'a VmarSpace,
    size: usize,
    align: usize,
) -> Result<(Vaddr, VmarCursorMut<'a>)> {
    let cur_cpu = guard.current_cpu();

    let assigned_range = assigned_range_on_cpu(cur_cpu);
    if !assigned_range.is_empty()
        && let Ok((va, cursor)) =
            try_lock_and_alloc_from(guard, vmspace, &assigned_range, size, align)
    {
        return Ok((va, cursor));
    }

    // Try other CPUs' ranges.
    if can_alloc_from_free_range(&assigned_range, size, align) {
        for cpu_id in all_cpus() {
            if cpu_id == cur_cpu {
                continue;
            }
            let other_range = assigned_range_on_cpu(cpu_id);
            if !other_range.is_empty()
                && let Ok((va, cursor)) =
                    try_lock_and_alloc_from(guard, vmspace, &other_range, size, align)
            {
                return Ok((va, cursor));
            }
        }
    }

    // Try the full userspace range.
    let full_range = VMAR_LOWEST_ADDR..VMAR_CAP_ADDR;
    try_lock_and_alloc_from(guard, vmspace, &full_range, size, align)
}

/// Allocates a virtual address range and gets a cursor covering the given range.
///
/// This function is similar to [`lock_and_alloc`], but it also ensures that
/// the returned cursor covers the given range.
pub fn lock_covering_range_and_alloc<'a>(
    guard: &'a DisabledPreemptGuard,
    vmspace: &'a VmarSpace,
    range: Range<Vaddr>,
    size: usize,
    align: usize,
) -> Result<(Vaddr, VmarCursorMut<'a>)> {
    let cpu_range = cpu_range_for_vaddr_range(&range);

    let start_va = assigned_range_on_cpu(*cpu_range.start()).start;
    let end_va = assigned_range_on_cpu(*cpu_range.end()).end;
    let range = start_va..end_va;

    if !range.is_empty()
        && let Ok((va, cursor)) = try_lock_and_alloc_from(guard, vmspace, &range, size, align)
    {
        return Ok((va, cursor));
    }

    // Try the full userspace range.
    let full_range = VMAR_LOWEST_ADDR..VMAR_CAP_ADDR;
    try_lock_and_alloc_from(guard, vmspace, &full_range, size, align)
}

fn try_lock_and_alloc_from<'a>(
    guard: &'a DisabledPreemptGuard,
    vmspace: &'a VmarSpace,
    range: &Range<Vaddr>,
    size: usize,
    align: usize,
) -> Result<(Vaddr, VmarCursorMut<'a>)> {
    if !can_alloc_from_free_range(range, size, align) {
        return Err(Error::with_message(
            Errno::ENOMEM,
            "the given range cannot fit the allocation",
        ));
    }
    let mut cursor = vmspace.cursor_mut(guard, range)?;
    let va = try_alloc_from_cursor_top_fast(&mut cursor, size, align)?;
    Ok((va, cursor))
}

/// Tries to allocate a virtual address range the cursor's guard range.
///
/// It firstly tries the top part of the first available free range. If not
/// fit, it searches top-down for a suitable range.
fn try_alloc_from_cursor_top_fast(
    cursor: &mut VmarCursorMut,
    size: usize,
    align: usize,
) -> Result<Vaddr> {
    let guard_range = cursor.guard_va_range();
    cursor.jump(guard_range.start).unwrap();

    if let Some(first_mapping) = find_next_mapped!(cursor, guard_range.end) {
        let free_end = first_mapping.map_to_addr();

        if let Ok(va) = alloc_from_free_range(guard_range.start..free_end, size, align) {
            return Ok(va);
        }
    } else {
        return Ok((guard_range.end - size).align_down(align));
    }

    // Search top-down for a suitable range.
    let mut search_end = guard_range.end;
    cursor.jump(search_end - PAGE_SIZE).unwrap();
    while (search_end - size).align_down(align) >= guard_range.start {
        let cur_level = cursor.level();
        let cur_va = cursor.virt_addr();

        let Some(meta) = cursor.aux_meta().inner.find_prev(&(cur_va + 1)) else {
            let cur_free_start = cursor.virt_addr().align_down(page_size_at(cur_level + 1));
            if let Ok(va) = alloc_from_free_range(cur_free_start..search_end, size, align) {
                return Ok(va);
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
            return Ok(va);
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
fn cpu_range_for_vaddr_range(range: &Range<Vaddr>) -> RangeInclusive<CpuId> {
    debug_assert!(range.start < range.end);

    let num_cpus = num_cpus();
    let per_cpu_range_size = assigned_range_size_for(num_cpus);

    let start_cpu = CpuId::try_from((range.start / per_cpu_range_size).min(num_cpus - 1)).unwrap();
    let end_cpu =
        CpuId::try_from(((range.end - 1) / per_cpu_range_size).min(num_cpus - 1)).unwrap();

    start_cpu..=end_cpu
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
