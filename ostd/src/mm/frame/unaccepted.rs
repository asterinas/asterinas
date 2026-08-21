// SPDX-License-Identifier: MPL-2.0

//! Early acceptance of unaccepted memory in Intel TDX guests.
//!
//! Before bringing up secondary CPUs, OSTD accepts the few ranges allocated
//! during early boot. Once all CPUs are running, each CPU accepts a disjoint
//! slice of the remaining EFI unaccepted-memory bitmap.

use core::{
    ops::Range,
    ptr::NonNull,
    sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering},
};

use align_ext::AlignExt;
use linux_boot_params::BootParams;
use spin::{Mutex, Once};
use tdx_guest::unaccepted_memory::{
    EfiUnacceptedMemory, LINUX_EFI_UNACCEPTED_MEM_TABLE_GUID,
    LINUX_EFI_UNACCEPTED_MEM_TABLE_VERSION,
};

use crate::{mm::Paddr, util::id_set::Id};

/// The singleton state for accepting memory described by the EFI unaccepted-memory table.
pub(crate) struct UnacceptedMemoryTable {
    table: AtomicPtr<EfiUnacceptedMemory>,
    finished_cpu_count: AtomicUsize,
    acceptance_failed: AtomicBool,
    early_allocated_ranges: Mutex<Option<(Range<Paddr>, Range<Paddr>)>>,
}

static UNACCEPTED_MEMORY_TABLE: Once<UnacceptedMemoryTable> = Once::new();

impl UnacceptedMemoryTable {
    /// Initializes the singleton from the EFI table referenced by Linux boot parameters.
    pub(crate) fn init(boot_params: &BootParams) {
        let result = UNACCEPTED_MEMORY_TABLE.try_call_once(|| {
            let table = parse_unaccepted_memory_table(boot_params).ok_or(())?;
            crate::info!("Found unaccepted memory table at {:p}", table.as_ptr());
            Ok::<Self, ()>(Self::new(table))
        });

        if result.is_err() {
            crate::warn!("Unaccepted memory table is unavailable");
        }
    }

    fn new(table: NonNull<EfiUnacceptedMemory>) -> Self {
        Self {
            table: AtomicPtr::new(table.as_ptr()),
            finished_cpu_count: AtomicUsize::new(0),
            acceptance_failed: AtomicBool::new(false),
            early_allocated_ranges: Mutex::new(None),
        }
    }

    pub(crate) fn singleton() -> Option<&'static Self> {
        UNACCEPTED_MEMORY_TABLE.get()
    }

    /// Rewrites the EFI table pointer to use the kernel linear mapping.
    ///
    /// This must be called on the BSP before secondary CPUs are started.
    pub(crate) fn remap_table_ptr_after_paging(&self) {
        let old_addr = self.table().as_ptr().addr();
        if old_addr < crate::mm::kspace::LINEAR_MAPPING_BASE_VADDR {
            let new_ptr = crate::mm::kspace::paddr_to_vaddr(old_addr) as *mut EfiUnacceptedMemory;
            self.table.store(new_ptr, Ordering::Release);
        }
    }

    /// Accepts memory that must be accessed before parallel early acceptance starts.
    pub(crate) fn accept_early_allocated_range(&self, start: Paddr, size: usize) {
        let end = start + size.align_up(crate::mm::PAGE_SIZE);

        // SAFETY: The table comes from EFI boot information. Before SMP startup
        // callers are serialized; the AP stack calls operate on disjoint ranges.
        unsafe {
            self.table()
                .as_ref()
                .accept_range_concurrent(start as u64, end as u64)
        }
        .unwrap_or_else(|err| panic!("failed to accept boot memory: {err:?}"));
    }

    /// Returns the physical range represented by the unaccepted-memory bitmap.
    pub(crate) fn acceptance_range(&self) -> Option<Range<Paddr>> {
        // SAFETY: The table comes from EFI boot information and remains valid
        // throughout early boot.
        let table = unsafe { self.table().as_ref() };
        let start = usize::try_from(table.phys_base()).ok()?;
        let end = usize::try_from(table.bitmap_coverage_end()?).ok()?;
        Some(start..end)
    }

    pub(crate) fn record_early_allocated_ranges(&self, ranges: (Range<Paddr>, Range<Paddr>)) {
        let mut early_allocated_ranges = self.early_allocated_ranges.lock();
        assert!(
            early_allocated_ranges.is_none(),
            "early allocated ranges already recorded"
        );
        *early_allocated_ranges = Some(ranges);
    }

    /// Consumes the state required to publish memory after parallel acceptance.
    pub(crate) fn take_publication_state(
        &self,
    ) -> Option<(Range<Paddr>, Range<Paddr>, Range<Paddr>)> {
        let acceptance_range = self.acceptance_range()?;
        let (range_1, range_2) = self.early_allocated_ranges.lock().take()?;
        Some((acceptance_range, range_1, range_2))
    }

    /// Accepts the current CPU's disjoint slice of the unaccepted-memory bitmap.
    pub(crate) fn accept_memory_slice_on_current_cpu(&self) {
        // SAFETY: The table remains valid throughout early boot.
        let table = unsafe { self.table().as_ref() };
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

        if start < end {
            // SAFETY: Every CPU receives a disjoint range of bitmap units.
            if let Err(err) = unsafe { table.accept_range_concurrent(start, end) } {
                crate::error!("CPU {cpu_id} failed to accept memory: {err:?}");
                self.acceptance_failed.store(true, Ordering::Release);
            }
        }
        self.finished_cpu_count.fetch_add(1, Ordering::Release);
    }

    /// Waits until all CPUs have completed their acceptance slices.
    pub(crate) fn wait_for_memory_acceptance(&self) {
        while self.finished_cpu_count.load(Ordering::Acquire) != crate::cpu::num_cpus() {
            assert!(
                !self.acceptance_failed.load(Ordering::Acquire),
                "parallel memory acceptance failed"
            );
            core::hint::spin_loop();
        }
        assert!(
            !self.acceptance_failed.load(Ordering::Acquire),
            "parallel memory acceptance failed"
        );
    }

    /// Publishes memory withheld until parallel early acceptance completed.
    pub(crate) fn publish_accepted_memory(&self) {
        super::allocator::publish_accepted_memory(self);
    }

    fn table(&self) -> NonNull<EfiUnacceptedMemory> {
        NonNull::new(self.table.load(Ordering::Acquire))
            .expect("initialized unaccepted-memory table pointer became null")
    }
}

/// Finds the unaccepted-memory table from EFI configuration tables.
fn parse_unaccepted_memory_table(boot_params: &BootParams) -> Option<NonNull<EfiUnacceptedMemory>> {
    let efi_info = boot_params.efi_info;
    let systab_addr = u64::from(efi_info.efi_systab) | (u64::from(efi_info.efi_systab_hi) << 32);

    if systab_addr == 0 {
        return None;
    }

    if !systab_addr.is_multiple_of(align_of::<uefi_raw::table::system::SystemTable>() as u64) {
        crate::warn!("EFI system table is misaligned");
        return None;
    }

    // SAFETY: `systab_addr` is firmware-provided, non-null, and suitably aligned. Paging is not
    // enabled, so the firmware physical address is directly accessible.
    let systab = unsafe { &*(systab_addr as *const uefi_raw::table::system::SystemTable) };

    let configuration_table = systab.configuration_table;
    if configuration_table.is_null() || !configuration_table.is_aligned() {
        crate::warn!("EFI configuration table is null or misaligned");
        return None;
    }

    let configuration_table_size = systab
        .number_of_configuration_table_entries
        .checked_mul(size_of::<uefi_raw::table::configuration::ConfigurationTable>());
    if configuration_table_size.is_none_or(|size| size > isize::MAX as usize) {
        crate::warn!("EFI configuration table is too large");
        return None;
    }

    // SAFETY: `configuration_table` is firmware-provided, non-null, suitably aligned, and its
    // total byte length has been checked to fit in a Rust slice.
    let entries = unsafe {
        core::slice::from_raw_parts(
            configuration_table,
            systab.number_of_configuration_table_entries,
        )
    };

    let table_ptr = entries
        .iter()
        .find(|entry| entry.vendor_guid == LINUX_EFI_UNACCEPTED_MEM_TABLE_GUID)?
        .vendor_table
        .cast::<EfiUnacceptedMemory>();

    if table_ptr.is_null() || !table_ptr.is_aligned() {
        return None;
    }

    // SAFETY: The pointer is non-null and aligned. The shared reference is only used to read
    // the firmware-provided header after those checks.
    let table = unsafe { table_ptr.as_ref()? };

    if table.version() != LINUX_EFI_UNACCEPTED_MEM_TABLE_VERSION {
        crate::warn!(
            "Unknown unaccepted memory table version: {}",
            table.version()
        );
        return None;
    }

    if table.unit_size_bytes() == 0 || !table.unit_size_bytes().is_power_of_two() {
        crate::warn!(
            "Invalid unaccepted memory table unit size: {}",
            table.unit_size_bytes()
        );
        return None;
    }

    let bitmap_addr = table_ptr
        .addr()
        .checked_add(size_of::<EfiUnacceptedMemory>())?;
    if !bitmap_addr.is_multiple_of(align_of::<AtomicU64>())
        || table.bitmap_size_bytes() == 0
        || !table
            .bitmap_size_bytes()
            .is_multiple_of(size_of::<AtomicU64>() as u64)
    {
        crate::warn!(
            "Invalid unaccepted memory table bitmap size: {}",
            table.bitmap_size_bytes()
        );
        return None;
    }

    let coverage_size = table.total_coverage_size()?;
    if table.phys_base().checked_add(coverage_size).is_none() {
        crate::warn!("Unaccepted memory table coverage overflows");
        return None;
    }

    NonNull::new(table_ptr)
}
