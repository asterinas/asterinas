// SPDX-License-Identifier: MPL-2.0

//! Unaccepted-memory management for Intel TDX guests.
//!
//! # Background
//!
//! Under Intel Trust Domain Extensions (TDX), physical memory assigned to the guest is initially
//! marked as *unaccepted*. To make physical memory usable, the kernel must explicitly accept it.
//!
//! # Memory Acceptance Modes
//!
//! This module supports two memory acceptance modes:
//!
//! 1. **Eager Mode**: Memory is partitioned evenly across all available CPUs during early boot,
//!    allowing APs to accept the entire physical address space in parallel before userspace starts.
//! 2. **Lazy Mode (Default)**: Accepting gigabytes of memory eagerly incurs severe boot latency.
//!    Usable memory is published to the frame allocator immediately. Each allocation is accepted
//!    on demand before being returned to the caller or its frames are initialized or accessed.

use core::{
    ops::Range,
    sync::atomic::{AtomicUsize, Ordering},
};

use linux_boot_params::{BootParams, EfiInfo};
use spin::Once;
use tdx_guest::unaccepted_memory::EfiUnacceptedMemory;

use crate::{
    boot::AcceptMemoryMode,
    cpu::CpuId,
    mm::Paddr,
    util::{
        id_set::Id,
        ops::{range_difference, range_intersection},
    },
};

/// Initializes the unaccepted-memory table from the EFI stub boot parameters.
pub(crate) fn init(boot_params: &BootParams) {
    let efi_info = boot_params.efi_info;
    assert_eq!(
        efi_info.efi_loader_signature,
        EfiInfo::TDX_LOADER_SIGNATURE,
        "TDX guests must be booted with `TDX_LOADER_SIGNATURE`"
    );
    assert_eq!(
        efi_info.efi_systab_hi, 0,
        "TDX unaccepted-memory table must be below 4 GiB"
    );

    let table_addr = efi_info.efi_systab;
    if table_addr == 0 {
        return;
    }
    let table_ptr =
        crate::mm::kspace::paddr_to_vaddr(table_addr as usize) as *const EfiUnacceptedMemory;

    crate::info!("Found unaccepted memory table at {:p}", table_ptr);
    // SAFETY:
    // - The table is safe to read because of the contract with the EFI stub.
    // - We reserve the table region in `table_memory_range`, so it will live as an immutable
    //   reference for `'static`.
    let table: &'static EfiUnacceptedMemory = unsafe { &*table_ptr };
    assert_ne!(
        table.unit_size_bytes(),
        0,
        "TDX unaccepted-memory table has zero unit size"
    );

    tdx_guest::init_acceptance_state(table);
}

/// Returns the physical range occupied by the table storage.
pub(crate) fn table_memory_range() -> Option<Range<Paddr>> {
    let table = tdx_guest::unaccepted_memory_table()?;
    let base = core::ptr::from_ref(table).addr() - crate::mm::kspace::LINEAR_MAPPING_BASE_VADDR;
    let size = size_of::<EfiUnacceptedMemory>() + table.bitmap_size_bytes() as usize;
    Some(base..base + size)
}

/// Accepts the BSP's slice of unaccepted memory, waits for all APs to finish,
/// and publishes the accepted memory to the global frame allocator.
///
/// # Safety
///
/// This function must be called only once in the boot context of the BSP, after booting APs.
pub(crate) unsafe fn accept_memory_on_bsp() {
    if crate::boot::early_cmdline().accept_memory_mode != AcceptMemoryMode::Eager {
        return;
    }

    let Some(table) = tdx_guest::unaccepted_memory_table() else {
        return;
    };

    let num_cpus = crate::cpu::num_cpus();
    let range = range_to_accept_on_cpu(table, CpuId::bsp().as_usize(), num_cpus);
    if !range.is_empty() {
        // SAFETY: The range is within the registered EFI table's coverage. The boot
        // contract guarantees that pending bits track unaccepted private memory,
        // while the remaining memory is already accepted.
        unsafe { ensure_accepted(range) };
    }
    FINISHED_CPU_COUNT.fetch_add(1, Ordering::Release);

    while FINISHED_CPU_COUNT.load(Ordering::Acquire) != num_cpus {
        core::hint::spin_loop();
    }
    publish_accepted_memory(table);
}

/// Accepts the calling AP's disjoint slice of unaccepted memory and marks completion.
///
/// # Safety
///
/// This function must be called only once per AP in its boot context.
pub(crate) unsafe fn accept_memory_on_ap() {
    if crate::boot::early_cmdline().accept_memory_mode != AcceptMemoryMode::Eager {
        return;
    }

    let Some(table) = tdx_guest::unaccepted_memory_table() else {
        return;
    };

    let num_cpus = crate::cpu::num_cpus();
    let range = range_to_accept_on_cpu(table, CpuId::current_racy().as_usize(), num_cpus);
    if !range.is_empty() {
        // SAFETY: The range is within the registered EFI table's coverage. The boot
        // contract guarantees that pending bits track unaccepted private memory,
        // while the remaining memory is already accepted.
        unsafe { ensure_accepted(range) };
    }
    FINISHED_CPU_COUNT.fetch_add(1, Ordering::Release);
}

/// Records the early-allocated ranges and returns the unaccepted-memory coverage range.
/// It may include memory that has already been accepted.
pub(super) fn init_unaccepted_memory_state(
    early_allocated_ranges: &[Range<Paddr>; 2],
) -> Range<Paddr> {
    if crate::boot::early_cmdline().accept_memory_mode != AcceptMemoryMode::Eager {
        return 0..0;
    }

    let Some(table) = tdx_guest::unaccepted_memory_table() else {
        return 0..0;
    };
    EARLY_ALLOCATED_RANGES.call_once(|| early_allocated_ranges.clone());

    let coverage_range = table.coverage_range();
    coverage_range.start as Paddr..coverage_range.end as Paddr
}

/// Ensures that all unaccepted-memory units overlapping `range` have been accepted.
///
/// # Panics
///
/// Panics if TDX memory acceptance fails.
///
/// # Safety
///
/// This function must only be called in a TDX guest after [`init`] has completed.
/// Any unaccepted memory in `range` must be tracked by pending bits in the registered
/// table; other memory in `range` must already be accepted.
/// Acceptance of the table's pending units must go through this API, and accepted
/// units must not become unaccepted again.
pub(super) unsafe fn ensure_accepted(range: Range<Paddr>) {
    let range = range.start as u64..range.end as u64;

    // SAFETY:
    // 1. The caller upholds the initialization, memory-state, and acceptance-protocol
    //    requirements. `init` obtains a valid table through the EFI boot contract.
    // 2. `crate::irq::disable_local` disables local interrupts and returns a guard that
    //   restores the previous interrupt state when dropped.
    unsafe { tdx_guest::ensure_accepted(range, crate::irq::disable_local) }
        .expect("failed to accept memory");
}

/// Publishes accepted memory regions to the global frame allocator.
fn publish_accepted_memory(table: &EfiUnacceptedMemory) {
    let coverage_range = table.coverage_range();
    let coverage_range = coverage_range.start as Paddr..coverage_range.end as Paddr;
    let early_allocated_ranges = EARLY_ALLOCATED_RANGES
        .get()
        .expect("early allocated ranges are unavailable");

    let regions = &crate::boot::EARLY_INFO.get().unwrap().memory_regions;
    let [early_range_1, early_range_2] = early_allocated_ranges;

    for region in regions
        .iter()
        .filter(|region| region.typ() == crate::boot::memory_region::MemoryRegionType::Usable)
    {
        let region_range = region.base()..region.end();
        for accepted_range in range_difference(&region_range, early_range_1)
            .flat_map(|range| range_difference(&range, early_range_2))
            .filter_map(|range| range_intersection(&range, &coverage_range))
        {
            crate::info!(
                "Adding accepted free frames to the allocator: {:x?}",
                accepted_range
            );
            super::allocator::get_global_frame_allocator()
                .add_free_memory(accepted_range.start, accepted_range.len());
        }
    }
}

/// Returns the physical address range assigned to one CPU within the table coverage.
fn range_to_accept_on_cpu(
    table: &EfiUnacceptedMemory,
    index: usize,
    num_cpus: usize,
) -> Range<Paddr> {
    let coverage_range = table.coverage_range();
    let unit_size = u64::from(table.unit_size_bytes());
    let num_units = (coverage_range.end - coverage_range.start) / unit_size;
    let units = partition_units(num_units, index, num_cpus);

    let start = (coverage_range.start + units.start * unit_size) as Paddr;
    let end = (coverage_range.start + units.end * unit_size) as Paddr;
    start..end
}

/// Returns one CPU's unit range, with partition sizes differing by at most one unit.
fn partition_units(total_units: u64, index: usize, num_cpus: usize) -> Range<u64> {
    debug_assert!(index < num_cpus);

    let index = index as u64;
    let num_cpus = num_cpus as u64;
    let units_per_cpu = total_units / num_cpus;
    let remainder = total_units % num_cpus;
    let start = units_per_cpu * index + index.min(remainder);
    let len = units_per_cpu + u64::from(index < remainder);
    start..start + len
}

static EARLY_ALLOCATED_RANGES: Once<[Range<Paddr>; 2]> = Once::new();
static FINISHED_CPU_COUNT: AtomicUsize = AtomicUsize::new(0);

#[cfg(ktest)]
mod test {
    use super::partition_units;
    use crate::prelude::ktest;

    #[ktest]
    fn partition_units_evenly() {
        assert_eq!(partition_units(32, 0, 4), 0..8);
        assert_eq!(partition_units(32, 1, 4), 8..16);
        assert_eq!(partition_units(32, 2, 4), 16..24);
        assert_eq!(partition_units(32, 3, 4), 24..32);
    }

    #[ktest]
    fn partition_units_with_remainder() {
        assert_eq!(partition_units(80, 0, 3), 0..27);
        assert_eq!(partition_units(80, 1, 3), 27..54);
        assert_eq!(partition_units(80, 2, 3), 54..80);
    }

    #[ktest]
    fn partition_units_partial_byte_tail() {
        assert_eq!(partition_units(83, 0, 3), 0..32);
        assert_eq!(partition_units(83, 1, 3), 32..56);
        assert_eq!(partition_units(83, 2, 3), 56..83);
    }

    #[ktest]
    fn partition_units_sparse_cpus() {
        assert_eq!(partition_units(19, 0, 4), 0..8);
        assert_eq!(partition_units(19, 1, 4), 8..16);
        assert_eq!(partition_units(19, 2, 4), 16..16);
        assert_eq!(partition_units(19, 3, 4), 16..19);
    }
}
