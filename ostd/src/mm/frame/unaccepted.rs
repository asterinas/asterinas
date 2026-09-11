// SPDX-License-Identifier: MPL-2.0

//! Unaccepted-memory management for Intel TDX guests.
//!
//! This module consumes the EFI unaccepted-memory table, accepts ranges needed
//! by early boot allocations, coordinates boot-time acceptance across CPUs, and
//! publishes accepted frames to the global frame allocator.

use core::{
    ops::Range,
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering},
};

use align_ext::AlignExt;
use linux_boot_params::BootParams;
use spin::Once;
use tdx_guest::{AcceptError, unaccepted_memory::EfiUnacceptedMemory};

use crate::{cpu::CpuId, mm::Paddr, util::id_set::Id};

/// Initializes the unaccepted-memory table from the EFI stub boot parameters.
pub(crate) fn init(boot_params: &BootParams) {
    let table_addr = boot_params.unaccepted_memory;
    if table_addr == 0 {
        crate::warn!("Unaccepted memory table is unavailable");
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

/// Records early allocated ranges and returns the physical range covered by the
/// unaccepted-memory bitmap, or `None` if unaccepted memory is unavailable.
pub(crate) fn init_unaccepted_range(
    under_4g_allocated_range: &Range<Paddr>,
    above_4g_allocated_range: &Range<Paddr>,
) -> Option<Range<Paddr>> {
    let table = UNACCEPTED_MEMORY_TABLE.get()?;
    table.early_allocated_ranges.call_once(|| {
        (
            under_4g_allocated_range.clone(),
            above_4g_allocated_range.clone(),
        )
    });
    Some(table.bitmap_coverage_range())
}

/// Accepts memory that must be accessed before parallel early acceptance starts.
pub(crate) fn accept_early_allocated_range(start: Paddr, size: usize) {
    if let Some(table) = UNACCEPTED_MEMORY_TABLE.get() {
        table.accept_early_allocated_range(start, size);
    }
}

static FINISHED_CPU_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Accepts the current CPU's disjoint slice of unaccepted memory and marks completion.
pub(crate) fn accept_memory_on_current_cpu() {
    let Some(table) = UNACCEPTED_MEMORY_TABLE.get() else {
        return;
    };
    let cpu_id = CpuId::current_racy();
    let num_cpus = crate::cpu::num_cpus();
    table
        .accept_cpu_slice(cpu_id, num_cpus)
        .expect("CPU failed to accept memory");
    FINISHED_CPU_COUNT.fetch_add(1, Ordering::Release);
}

/// Coordinates acceptance on the BSP: accepts the BSP's slice, waits for all APs,
/// and publishes the accepted memory to the global frame allocator.
pub(crate) fn accept_memory_on_bsp() {
    let Some(table) = UNACCEPTED_MEMORY_TABLE.get() else {
        return;
    };
    accept_memory_on_current_cpu();
    let num_cpus = crate::cpu::num_cpus();
    while FINISHED_CPU_COUNT.load(Ordering::Acquire) != num_cpus {
        core::hint::spin_loop();
    }
    table.publish_accepted_memory();
}

/// Manages the EFI unaccepted-memory table and operations.
struct UnacceptedMemoryTable {
    table_ptr: NonNull<EfiUnacceptedMemory>,
    early_allocated_ranges: Once<(Range<Paddr>, Range<Paddr>)>,
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

        let table = self.table();
        let cpu_id = cpu_id.as_usize() as u64;
        let num_cpus = num_cpus as u64;
        let unit_size = table.unit_size_bytes() as u64;
        let coverage = table.bitmap_coverage_range();

        let num_units = (coverage.end - coverage.start) / unit_size;
        let units_per_cpu = num_units / num_cpus;
        let remainder = num_units % num_cpus;
        let start_units = units_per_cpu * cpu_id + remainder.min(cpu_id);
        let end_cpu_id = cpu_id + 1;
        let end_units = units_per_cpu * end_cpu_id + remainder.min(end_cpu_id);
        let start = coverage.start + start_units * unit_size;
        let end = coverage.start + end_units * unit_size;

        if start < end {
            // SAFETY: Every CPU receives a disjoint range of bitmap units.
            unsafe { table.accept_range(start, end)? };
        }
        Ok(())
    }

    /// Publishes accepted memory regions to the global frame allocator.
    fn publish_accepted_memory(&self) {
        let coverage_range = self.bitmap_coverage_range();
        let (under_4g_allocated_range, above_4g_allocated_range) = self
            .early_allocated_ranges
            .get()
            .expect("early allocated ranges are unavailable");

        super::allocator::for_each_free_boot_range(
            under_4g_allocated_range,
            above_4g_allocated_range,
            |free_range| {
                let start = free_range.start.max(coverage_range.start);
                let end = free_range.end.min(coverage_range.end);
                if start < end {
                    crate::info!(
                        "Adding accepted free frames to the allocator: {:x?}",
                        start..end
                    );
                    super::allocator::get_global_frame_allocator()
                        .add_free_memory(start, (start..end).len());
                }
            },
        );
    }

    /// Accepts memory that must be accessed before parallel early acceptance starts.
    fn accept_early_allocated_range(&self, start: Paddr, size: usize) {
        let start_aligned = start.align_down(crate::mm::PAGE_SIZE);
        let end_aligned = (start + size).align_up(crate::mm::PAGE_SIZE);

        // SAFETY: The table comes from EFI boot information. Before SMP startup
        // callers are serialized; the AP stack calls operate on disjoint ranges.
        unsafe {
            self.table()
                .accept_range(start_aligned as u64, end_aligned as u64)
        }
        .expect("failed to accept boot memory");
    }

    /// Returns the physical range represented by the unaccepted-memory bitmap.
    fn bitmap_coverage_range(&self) -> Range<Paddr> {
        let range = self.table().bitmap_coverage_range();
        range.start as Paddr..range.end as Paddr
    }

    /// Returns a shared reference to the EFI unaccepted-memory table.
    fn table(&self) -> &EfiUnacceptedMemory {
        // SAFETY: The EFI stub allocated and initialized this table, and its
        // physical address was converted to the kernel linear mapping in `init`.
        unsafe { self.table_ptr.as_ref() }
    }
}
