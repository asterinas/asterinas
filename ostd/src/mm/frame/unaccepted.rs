// SPDX-License-Identifier: MPL-2.0

//! Unaccepted-memory management for Intel TDX guests.

use core::{
    ops::Range,
    sync::atomic::{AtomicUsize, Ordering},
};

use align_ext::AlignExt;
use linux_boot_params::{BootParams, EfiInfo};
use spin::Once;
use tdx_guest::unaccepted_memory::EfiUnacceptedMemory;

use crate::{cpu::CpuId, mm::Paddr, util::id_set::Id};

/// Initializes the unaccepted-memory table from the EFI stub boot parameters.
pub(crate) fn init(boot_params: &BootParams) {
    let efi_info = boot_params.efi_info;
    assert_eq!(
        efi_info.efi_loader_signature,
        EfiInfo::ASTERINAS_LOADER_SIGNATURE,
        "TDX guests must be booted by the Asterinas EFI stub"
    );
    assert_eq!(
        efi_info.efi_systab_hi, 0,
        "Asterinas unaccepted-memory table must be below 4 GiB"
    );

    let table_addr = efi_info.efi_systab;
    if table_addr == 0 {
        return;
    }
    let table_ptr =
        crate::mm::kspace::paddr_to_vaddr(table_addr as usize) as *const EfiUnacceptedMemory;

    crate::info!("Found unaccepted memory table at {:p}", table_ptr);
    // SAFETY: The EFI stub provides a valid table that remains alive for the kernel lifetime.
    let table: &'static EfiUnacceptedMemory = unsafe { &*table_ptr };
    UNACCEPTED_TABLE.call_once(|| table);
}

/// Returns the physical range occupied by the table storage.
pub(crate) fn table_memory_range() -> Option<Range<Paddr>> {
    let table = UNACCEPTED_TABLE.get().copied()?;
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
    let Some(table) = UNACCEPTED_TABLE.get().copied() else {
        return;
    };

    let num_cpus = crate::cpu::num_cpus();
    let range = range_to_accept_on_cpu(table, CpuId::bsp().as_usize(), num_cpus);
    if !range.is_empty() {
        // SAFETY:
        // - This table and bitmap describe pending private-memory units.
        // - The BSP and the APs accept pages concurrently, but their ranges are disjoint.
        unsafe { table.accept_range(range.start, range.end) }.expect("failed to accept memory");
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
/// This function must be called only once in the boot context of the AP.
pub(crate) unsafe fn accept_memory_on_ap() {
    let Some(table) = UNACCEPTED_TABLE.get().copied() else {
        return;
    };

    let num_cpus = crate::cpu::num_cpus();
    let range = range_to_accept_on_cpu(table, CpuId::current_racy().as_usize(), num_cpus);
    if !range.is_empty() {
        // SAFETY:
        // - This table and bitmap describe pending private-memory units.
        // - The BSP and the APs accept pages concurrently, but their ranges are disjoint.
        unsafe { table.accept_range(range.start, range.end) }.expect("failed to accept memory");
    }
    FINISHED_CPU_COUNT.fetch_add(1, Ordering::Release);
}

/// Records early-allocated ranges and returns the unaccepted-memory table's coverage range.
/// It may include memory that has already been accepted.
pub(super) fn init_unaccepted_memory_state(
    early_allocated_ranges: &[Range<Paddr>; 2],
) -> Range<Paddr> {
    let Some(table) = UNACCEPTED_TABLE.get().copied() else {
        return 0..0;
    };
    EARLY_ALLOCATED_RANGES.call_once(|| early_allocated_ranges.clone());

    let coverage_range = table.coverage_range();
    coverage_range.start as Paddr..coverage_range.end as Paddr
}

/// Accepts memory that must be accessed before parallel early acceptance starts.
///
/// # Safety
///
/// This function must not be called concurrently with any other functions that accept pages.
pub(super) unsafe fn accept_early_allocated_range(start: Paddr, size: usize) {
    let Some(table) = UNACCEPTED_TABLE.get().copied() else {
        return;
    };
    let start_aligned = start.align_down(crate::mm::PAGE_SIZE);
    let end_aligned = (start + size).align_up(crate::mm::PAGE_SIZE);

    // SAFETY:
    // - This table and bitmap describe pending private-memory units.
    // - There are no concurrent operations, as guaranteed by the caller.
    unsafe { table.accept_range(start_aligned as u64, end_aligned as u64) }
        .expect("failed to accept boot memory");
}

/// Publishes accepted memory regions to the global frame allocator.
fn publish_accepted_memory(table: &EfiUnacceptedMemory) {
    let coverage_range = table.coverage_range();
    let early_allocated_ranges = EARLY_ALLOCATED_RANGES
        .get()
        .expect("early allocated ranges are unavailable");

    let accepted_ranges = super::allocator::usable_boot_ranges_excluding(early_allocated_ranges)
        .filter_map(|range| {
            let start = range.start.max(coverage_range.start as Paddr);
            let end = range.end.min(coverage_range.end as Paddr);
            (start < end).then_some(start..end)
        });

    for range in accepted_ranges {
        crate::info!("Adding accepted free frames to the allocator: {:x?}", range);
        super::allocator::get_global_frame_allocator().add_free_memory(range.start, range.len());
    }
}

/// Returns the GPA range assigned to one CPU within the table coverage.
fn range_to_accept_on_cpu(
    table: &EfiUnacceptedMemory,
    index: usize,
    num_cpus: usize,
) -> Range<u64> {
    debug_assert!(index < num_cpus);

    let coverage_range = table.coverage_range();
    let unit_size = u64::from(table.unit_size_bytes());
    let num_units = (coverage_range.end - coverage_range.start) / unit_size;
    let (start_unit, end_unit) = partition_units(num_units, index, num_cpus);

    let start = coverage_range.start + start_unit * unit_size;
    let end = coverage_range.start + end_unit * unit_size;
    start..end
}

/// Uniformly partitions `total_units` across `num_cpus`, aligned to 8 units (1 bitmap byte).
fn partition_units(total_units: u64, index: usize, num_cpus: usize) -> (u64, u64) {
    const UNITS_PER_BYTE: u64 = 8;

    let total_bytes = total_units / UNITS_PER_BYTE;
    let base_bytes = total_bytes / num_cpus as u64;
    let remain_bytes = total_bytes % num_cpus as u64;

    let index = index as u64;
    let start_byte = base_bytes * index + remain_bytes.min(index);
    let end_byte = base_bytes * (index + 1) + remain_bytes.min(index + 1);

    let start_unit = start_byte * UNITS_PER_BYTE;
    let mut end_unit = end_byte * UNITS_PER_BYTE;
    if index == (num_cpus as u64 - 1) {
        end_unit = total_units;
    }
    (start_unit, end_unit)
}

static UNACCEPTED_TABLE: Once<&'static EfiUnacceptedMemory> = Once::new();
static EARLY_ALLOCATED_RANGES: Once<[Range<Paddr>; 2]> = Once::new();
static FINISHED_CPU_COUNT: AtomicUsize = AtomicUsize::new(0);
