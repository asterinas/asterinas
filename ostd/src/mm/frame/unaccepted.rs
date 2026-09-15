// SPDX-License-Identifier: MPL-2.0

//! Unaccepted-memory management for Intel TDX guests.
//!
//! This module consumes the EFI unaccepted-memory table, accepts ranges needed
//! by early boot allocations, coordinates boot-time acceptance across CPUs, and
//! publishes accepted frames to the global frame allocator.

use core::{
    ops::{Deref, Range},
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering},
};

use align_ext::AlignExt;
use linux_boot_params::{BootParams, EfiInfo};
use spin::Once;
use tdx_guest::{AcceptError, unaccepted_memory::EfiUnacceptedMemory};

use crate::{cpu::CpuId, mm::Paddr, util::id_set::Id};

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
    UNACCEPTED_MEMORY_TABLE.call_once(|| UnacceptedMemoryTable {
        table_ptr,
        early_allocated_ranges: Once::new(),
    });
}

/// Stores the early allocated ranges for later frame publication and returns
/// the physical range covered by the unaccepted-memory bitmap.
pub(crate) fn register_unaccepted_range(
    early_allocated_ranges: &[Range<Paddr>; 2],
) -> Option<Range<Paddr>> {
    let table = UNACCEPTED_MEMORY_TABLE.get()?;
    table
        .early_allocated_ranges
        .call_once(|| early_allocated_ranges.clone());
    let range = table.bitmap_coverage_range();
    Some(range.start as Paddr..range.end as Paddr)
}

/// Accepts memory that must be accessed before parallel early acceptance starts.
pub(crate) fn accept_early_allocated_range(start: Paddr, size: usize) {
    let Some(table) = UNACCEPTED_MEMORY_TABLE.get() else {
        return;
    };
    let start_aligned = start.align_down(crate::mm::PAGE_SIZE);
    let end_aligned = (start + size).align_up(crate::mm::PAGE_SIZE);

    // SAFETY: The table comes from EFI boot information. Before SMP startup
    // callers are serialized; the AP stack calls operate on disjoint ranges.
    unsafe { table.accept_range(start_aligned as u64, end_aligned as u64) }
        .expect("failed to accept boot memory");
}

static FINISHED_CPU_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Accepts the calling AP's disjoint slice of unaccepted memory and marks completion.
///
/// This function must be called only once per AP in its boot context.
pub(crate) fn accept_memory_on_ap() {
    let Some(table) = UNACCEPTED_MEMORY_TABLE.get() else {
        return;
    };
    let cpu_id = CpuId::current_racy();
    let num_cpus = crate::cpu::num_cpus();
    table
        .accept_cpu_slice(cpu_id, num_cpus)
        .expect("AP failed to accept memory");
    FINISHED_CPU_COUNT.fetch_add(1, Ordering::Release);
}

/// Accepts the BSP's slice of unaccepted memory, waits for all APs to finish,
/// and publishes the accepted memory to the global frame allocator.
///
/// This function must be called only once on the BSP in its boot context
/// after booting all APs.
pub(crate) fn accept_memory_on_bsp() {
    let Some(table) = UNACCEPTED_MEMORY_TABLE.get() else {
        return;
    };
    let num_cpus = crate::cpu::num_cpus();
    table
        .accept_cpu_slice(CpuId::bsp(), num_cpus)
        .expect("BSP failed to accept memory");
    FINISHED_CPU_COUNT.fetch_add(1, Ordering::Release);

    while FINISHED_CPU_COUNT.load(Ordering::Acquire) != num_cpus {
        core::hint::spin_loop();
    }
    table.publish_accepted_memory();
}

/// Manages the EFI unaccepted-memory table and operations.
struct UnacceptedMemoryTable {
    table_ptr: NonNull<EfiUnacceptedMemory>,
    early_allocated_ranges: Once<[Range<Paddr>; 2]>,
}

// SAFETY: `table_ptr` is initialized once before this value is published and
// points to an EFI allocation that remains valid for the kernel lifetime. The
// table's concurrent bitmap operations are synchronized by their API.
unsafe impl Send for UnacceptedMemoryTable {}
unsafe impl Sync for UnacceptedMemoryTable {}

static UNACCEPTED_MEMORY_TABLE: Once<UnacceptedMemoryTable> = Once::new();

impl UnacceptedMemoryTable {
    /// Accepts the slice of memory assigned to `cpu_id` out of `num_cpus`.
    fn accept_cpu_slice(&self, cpu_id: CpuId, num_cpus: usize) -> Result<(), AcceptError> {
        debug_assert!(cpu_id.as_usize() < num_cpus);

        let unit_size = self.unit_size_bytes() as u64;
        let coverage = self.bitmap_coverage_range();

        let num_units = (coverage.end - coverage.start) / unit_size;
        let (start_unit, end_unit) = partition_range(num_units, cpu_id.as_usize(), num_cpus);

        let start = coverage.start + start_unit * unit_size;
        let end = coverage.start + end_unit * unit_size;

        if start < end {
            // SAFETY: Every CPU receives a disjoint range of bitmap units.
            unsafe { self.accept_range(start, end)? };
        }
        Ok(())
    }

    /// Publishes accepted memory regions to the global frame allocator.
    fn publish_accepted_memory(&self) {
        let range = self.bitmap_coverage_range();
        let coverage_range = range.start as Paddr..range.end as Paddr;
        let early_allocated_ranges = self
            .early_allocated_ranges
            .get()
            .expect("early allocated ranges are unavailable");

        let accepted_ranges = super::allocator::free_boot_ranges(early_allocated_ranges)
            .filter_map(|range| {
                let start = range.start.max(coverage_range.start);
                let end = range.end.min(coverage_range.end);
                (start < end).then_some(start..end)
            });

        for range in accepted_ranges {
            crate::info!("Adding accepted free frames to the allocator: {:x?}", range);
            super::allocator::get_global_frame_allocator()
                .add_free_memory(range.start, range.len());
        }
    }
}

impl Deref for UnacceptedMemoryTable {
    type Target = EfiUnacceptedMemory;

    fn deref(&self) -> &Self::Target {
        // SAFETY: The EFI stub allocated and initialized this table, and its
        // physical address was converted to the kernel linear mapping in `init`.
        unsafe { self.table_ptr.as_ref() }
    }
}

/// Uniformly partitions `total` units across `num_parts` parts, returning `[start, end)` for `index`.
fn partition_range(total: u64, index: usize, num_parts: usize) -> (u64, u64) {
    let index = index as u64;
    let num_parts = num_parts as u64;

    let base = total / num_parts;
    let rem = total % num_parts;

    let start = base * index + rem.min(index);
    let end = base * (index + 1) + rem.min(index + 1);
    (start, end)
}
