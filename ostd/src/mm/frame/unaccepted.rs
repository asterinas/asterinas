// SPDX-License-Identifier: MPL-2.0

//! Unaccepted-memory management for Intel TDX guests.

use core::{
    ops::Range,
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering},
};

use align_ext::AlignExt;
use linux_boot_params::{BootParams, EfiInfo};
use spin::Once;
use tdx_guest::unaccepted_memory::EfiUnacceptedMemory;

use crate::{
    boot::memory_region::{MemoryRegion, MemoryRegionType},
    cpu::CpuId,
    mm::Paddr,
    util::{id_set::Id, ops::range_difference},
};

/// Initializes the unaccepted-memory table from the EFI stub boot parameters.
pub(crate) fn init(boot_params: &BootParams) {
    let efi_info = boot_params.efi_info;
    if efi_info.efi_loader_signature != EfiInfo::ASTERINAS_LOADER_SIGNATURE {
        return;
    }
    let table_addr = efi_info.efi_systab;
    if table_addr == 0 {
        return;
    }

    let table_ptr = NonNull::new(
        crate::mm::kspace::paddr_to_vaddr(table_addr as usize) as *mut EfiUnacceptedMemory
    )
    .expect("unaccepted-memory table address is null");

    crate::info!("Found unaccepted memory table at {:p}", table_ptr.as_ptr());
    // SAFETY: The EFI stub provides a valid table that remains alive for the kernel lifetime.
    let table: &'static EfiUnacceptedMemory = unsafe { table_ptr.as_ref() };
    UNACCEPTED_TABLE.call_once(|| table);
}

/// Returns the table storage region to reserve before initializing frame allocators.
pub(crate) fn table_memory_region() -> Option<MemoryRegion> {
    let table = load_unaccepted_table()?;
    let base = core::ptr::from_ref(table).addr() - crate::mm::kspace::LINEAR_MAPPING_BASE_VADDR;
    let size = size_of::<EfiUnacceptedMemory>() + table.bitmap_size_bytes() as usize;
    Some(MemoryRegion::new(base, size, MemoryRegionType::Reserved))
}

/// Accepts the BSP's slice of unaccepted memory, waits for all APs to finish,
/// and publishes the accepted memory to the global frame allocator.
///
/// This function must be called only once on the BSP in its boot context
/// after booting all APs.
pub(crate) fn accept_memory_on_bsp() {
    let Some(table) = accept_memory_on_cpu(CpuId::bsp()) else {
        return;
    };
    while FINISHED_CPU_COUNT.load(Ordering::Acquire) != crate::cpu::num_cpus() {
        core::hint::spin_loop();
    }
    publish_accepted_memory(table);
}

/// Accepts the calling AP's disjoint slice of unaccepted memory and marks completion.
///
/// This function must be called only once per AP in its boot context.
pub(crate) fn accept_memory_on_ap() {
    accept_memory_on_cpu(CpuId::current_racy());
}

/// Initializes the frame allocator's memory managed by the unaccepted-memory table.
pub(super) fn init_allocator_memory(early_allocated_ranges: &[Range<Paddr>; 2]) {
    if let Some(table) = load_unaccepted_table() {
        EARLY_ALLOCATED_RANGES.call_once(|| early_allocated_ranges.clone());
        let coverage_range = table.coverage_range();
        add_free_ranges(
            super::allocator::free_boot_ranges(early_allocated_ranges).flat_map(move |range| {
                range_difference(
                    &range,
                    &(coverage_range.start as Paddr..coverage_range.end as Paddr),
                )
            }),
        );
    } else {
        add_free_ranges(super::allocator::free_boot_ranges(early_allocated_ranges));
    }
}

/// Accepts memory that must be accessed before parallel early acceptance starts.
pub(super) fn accept_early_allocated_range(start: Paddr, size: usize) {
    let Some(table) = load_unaccepted_table() else {
        return;
    };
    let start_aligned = start.align_down(crate::mm::PAGE_SIZE);
    let end_aligned = (start + size).align_up(crate::mm::PAGE_SIZE);

    // SAFETY: Before SMP startup callers are serialized; the AP stack calls operate on disjoint ranges.
    unsafe { table.accept_range(start_aligned as u64, end_aligned as u64) }
        .expect("failed to accept boot memory");
}

fn add_free_ranges(ranges: impl Iterator<Item = Range<Paddr>>) {
    for range in ranges {
        crate::info!("Adding free frames to the allocator: {:x?}", range);
        super::allocator::get_global_frame_allocator().add_free_memory(range.start, range.len());
    }
}

/// Accepts the slice of memory assigned to a CPU and marks completion.
fn accept_memory_on_cpu(cpu_id: CpuId) -> Option<&'static EfiUnacceptedMemory> {
    let table = load_unaccepted_table()?;
    let num_cpus = crate::cpu::num_cpus();
    debug_assert!(cpu_id.as_usize() < num_cpus);

    let unit_size = u64::from(table.unit_size_bytes());
    let coverage_range = table.coverage_range();

    let num_units = (coverage_range.end - coverage_range.start) / unit_size;
    let (start_unit, end_unit) = partition_units(num_units, cpu_id.as_usize(), num_cpus);

    let start = coverage_range.start + start_unit * unit_size;
    let end = coverage_range.start + end_unit * unit_size;

    if start < end {
        // SAFETY: Every CPU receives a disjoint range of bitmap units.
        unsafe { table.accept_range(start, end) }.expect("failed to accept memory");
    }
    FINISHED_CPU_COUNT.fetch_add(1, Ordering::Release);
    Some(table)
}

/// Publishes accepted memory regions to the global frame allocator.
fn publish_accepted_memory(table: &EfiUnacceptedMemory) {
    let coverage_range = table.coverage_range();
    let early_allocated_ranges = EARLY_ALLOCATED_RANGES
        .get()
        .expect("early allocated ranges are unavailable");

    let accepted_ranges =
        super::allocator::free_boot_ranges(early_allocated_ranges).filter_map(|range| {
            let start = range.start.max(coverage_range.start as Paddr);
            let end = range.end.min(coverage_range.end as Paddr);
            (start < end).then_some(start..end)
        });

    for range in accepted_ranges {
        crate::info!("Adding accepted free frames to the allocator: {:x?}", range);
        super::allocator::get_global_frame_allocator().add_free_memory(range.start, range.len());
    }
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

fn load_unaccepted_table() -> Option<&'static EfiUnacceptedMemory> {
    UNACCEPTED_TABLE.get().copied()
}

static UNACCEPTED_TABLE: Once<&'static EfiUnacceptedMemory> = Once::new();
static EARLY_ALLOCATED_RANGES: Once<[Range<Paddr>; 2]> = Once::new();
static FINISHED_CPU_COUNT: AtomicUsize = AtomicUsize::new(0);
