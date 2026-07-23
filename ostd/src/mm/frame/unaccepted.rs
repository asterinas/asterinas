// SPDX-License-Identifier: MPL-2.0

//! Early acceptance of unaccepted memory in Intel TDX guests.
//!
//! The EFI stub accepts memory required to enter the kernel. Before bringing
//! up secondary CPUs, OSTD accepts the few additional ranges needed for SMP
//! startup. Once all CPUs are running, each CPU accepts a disjoint slice of the
//! remaining EFI unaccepted-memory bitmap.

use core::{
    ops::Range,
    ptr::NonNull,
    sync::atomic::{AtomicPtr, AtomicUsize, Ordering},
};

use tdx_guest::unaccepted_memory::EfiUnacceptedMemory;

use crate::util::id_set::Id;

/// Sets the unaccepted-memory table pointer parsed at boot entry.
pub(crate) fn set_unaccepted_memory_table(table: Option<NonNull<EfiUnacceptedMemory>>) {
    let Some(table) = table else {
        crate::warn!("Unaccepted memory table is unavailable");
        return;
    };

    crate::info!("Found unaccepted memory table at {:p}", table.as_ptr());
    UNACCEPTED_TABLE.store(table.as_ptr(), Ordering::Release);
}

/// Rewrites the EFI table pointer to use the kernel linear mapping.
///
/// This must be called on the BSP before secondary CPUs are started.
pub(crate) fn remap_table_ptr_after_paging() {
    let Some(table) = get_unaccepted_memory_table() else {
        return;
    };

    let old_addr = table.as_ptr().addr();
    if old_addr < crate::mm::kspace::LINEAR_MAPPING_BASE_VADDR {
        let new_ptr = crate::mm::kspace::paddr_to_vaddr(old_addr) as *mut EfiUnacceptedMemory;
        UNACCEPTED_TABLE.store(new_ptr, Ordering::Release);
    }
}

/// Accepts memory that must be accessed before parallel early acceptance starts.
pub(crate) fn accept_early_allocated_range(start: u64, end: u64) {
    let Some(table) = get_unaccepted_memory_table() else {
        return;
    };

    // SAFETY: The table comes from EFI boot information. Before SMP startup
    // callers are serialized; the AP stack calls operate on disjoint ranges.
    unsafe { table.as_ref().accept_range_concurrent(start, end) }
        .unwrap_or_else(|err| panic!("failed to accept boot memory: {err:?}"));
}

/// Returns the physical range represented by the unaccepted-memory bitmap.
pub(crate) fn get_acceptance_range() -> Option<Range<crate::mm::Paddr>> {
    let table = get_unaccepted_memory_table()?;
    // SAFETY: The table comes from EFI boot information and remains valid
    // throughout early boot.
    let table = unsafe { table.as_ref() };
    let start = usize::try_from(table.phys_base()).ok()?;
    let end = usize::try_from(table.bitmap_coverage_end()?).ok()?;
    Some(start..end)
}

/// Accepts the current CPU's disjoint slice of the unaccepted-memory bitmap.
pub(crate) fn accept_memory_slice_on_current_cpu() {
    let Some(table) = get_unaccepted_memory_table() else {
        FINISHED_CPU_COUNT.fetch_add(1, Ordering::Release);
        return;
    };

    // SAFETY: The table remains valid throughout early boot.
    let table = unsafe { table.as_ref() };
    let cpu_id = crate::cpu::CpuId::current_racy().as_usize() as u64;
    let num_cpus = crate::cpu::num_cpus() as u64;
    let unit_size = u64::from(table.unit_size_bytes());
    let coverage_start = table.phys_base();
    let coverage_end = table
        .bitmap_coverage_end()
        .expect("unaccepted-memory bitmap coverage overflowed");
    let num_units = (coverage_end - coverage_start) / unit_size;
    let units_per_cpu = num_units / num_cpus;
    let remainder = num_units % num_cpus;
    let start_units = units_per_cpu * cpu_id + remainder.min(cpu_id);
    let end_cpu_id = cpu_id + 1;
    let end_units = units_per_cpu * end_cpu_id + remainder.min(end_cpu_id);
    let start = coverage_start + start_units * unit_size;
    let end = coverage_start + end_units * unit_size;

    if start >= end {
        FINISHED_CPU_COUNT.fetch_add(1, Ordering::Release);
        return;
    }

    // SAFETY: Every CPU receives a disjoint range of bitmap units.
    unsafe { table.accept_range_concurrent(start, end) }
        .unwrap_or_else(|err| panic!("CPU {cpu_id} failed to accept memory: {err:?}"));
    FINISHED_CPU_COUNT.fetch_add(1, Ordering::Release);
}

/// Waits until all CPUs have completed their acceptance slices.
pub(crate) fn wait_for_memory_acceptance() {
    while FINISHED_CPU_COUNT.load(Ordering::Acquire) != crate::cpu::num_cpus() {
        core::hint::spin_loop();
    }
}

fn get_unaccepted_memory_table() -> Option<NonNull<EfiUnacceptedMemory>> {
    NonNull::new(UNACCEPTED_TABLE.load(Ordering::Acquire))
}

static UNACCEPTED_TABLE: AtomicPtr<EfiUnacceptedMemory> = AtomicPtr::new(core::ptr::null_mut());
static FINISHED_CPU_COUNT: AtomicUsize = AtomicUsize::new(0);
