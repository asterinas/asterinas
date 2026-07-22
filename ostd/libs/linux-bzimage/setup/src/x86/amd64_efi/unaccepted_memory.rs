// SPDX-License-Identifier: MPL-2.0

//! EFI-side bitmap management for TDX unaccepted memory.
//!
//! This module builds and installs the Linux-compatible unaccepted-memory table,
//! records unaccepted ranges in the bitmap, and accepts boot-critical ranges
//! before transferring control to the kernel.

use core::{mem::size_of, ptr::NonNull};

use align_ext::AlignExt;
use tdx_guest::unaccepted_memory::{
    EFI_UNACCEPTED_UNIT_SIZE, EfiUnacceptedMemory, LINUX_EFI_UNACCEPTED_MEM_TABLE_GUID,
    LINUX_EFI_UNACCEPTED_MEM_TABLE_VERSION,
};
use uefi::{
    boot::{AllocateType, MemoryType},
    mem::memory_map::MemoryMap,
};

use crate::x86::amd64_efi::efi::PAGE_SIZE;

pub(crate) fn find_unaccepted_table() -> Option<&'static mut EfiUnacceptedMemory> {
    let memory_map = uefi::boot::memory_map(MemoryType::LOADER_DATA).ok()?;
    let st = uefi::table::system_table_raw()?;
    let system_table = st.as_ptr();

    // SAFETY: EFI system table pointer is firmware-owned and valid at this phase.
    let config_table = unsafe { (*system_table).configuration_table };
    // SAFETY: EFI system table pointer is firmware-owned and valid at this phase.
    let num_entries = unsafe { (*system_table).number_of_configuration_table_entries };

    if config_table.is_null() || num_entries == 0 {
        return None;
    }

    // SAFETY: EFI config table pointer/length come from firmware and are valid here.
    let entries = unsafe { core::slice::from_raw_parts(config_table, num_entries) };

    let entry = entries
        .iter()
        .find(|entry| entry.vendor_guid == LINUX_EFI_UNACCEPTED_MEM_TABLE_GUID)?;

    let Some(mut ptr) = NonNull::new(entry.vendor_table.cast::<EfiUnacceptedMemory>()) else {
        crate::println!("[EFI stub] warning: unaccepted memory table pointer is null");
        return None;
    };
    if !ptr.as_ptr().is_aligned() {
        crate::println!("[EFI stub] warning: unaccepted memory table pointer is misaligned");
        return None;
    }

    let table_addr = u64::try_from(ptr.as_ptr().addr()).ok()?;
    if !is_memory_range_valid(&memory_map, table_addr, size_of::<EfiUnacceptedMemory>()) {
        crate::println!("[EFI stub] warning: unaccepted memory table header is outside EFI memory");
        return None;
    }

    // SAFETY: The table header lies within a suitable EFI memory descriptor.
    let version = unsafe { ptr.as_ref().version() };
    if version != LINUX_EFI_UNACCEPTED_MEM_TABLE_VERSION {
        crate::println!(
            "[EFI stub] warning: unknown unaccepted memory table version: {}",
            version
        );
        return None;
    }

    // SAFETY: The table header lies within a suitable EFI memory descriptor.
    let (unit_size, bitmap_size) = unsafe {
        let table = ptr.as_ref();
        (table.unit_size_bytes(), table.bitmap_size_bytes())
    };
    if u64::from(unit_size) != EFI_UNACCEPTED_UNIT_SIZE {
        crate::println!(
            "[EFI stub] warning: unaccepted memory table has invalid unit size: {}",
            unit_size
        );
        return None;
    }
    if bitmap_size == 0 || !bitmap_size.is_multiple_of(8) {
        crate::println!(
            "[EFI stub] warning: unaccepted memory table has invalid bitmap size: {}",
            bitmap_size
        );
        return None;
    }

    let table_size = u64::try_from(size_of::<EfiUnacceptedMemory>())
        .ok()?
        .checked_add(bitmap_size)?;
    if !is_memory_range_valid(&memory_map, table_addr, usize::try_from(table_size).ok()?) {
        crate::println!("[EFI stub] warning: unaccepted memory table extends outside EFI memory");
        return None;
    }

    // SAFETY: The table header lies within a suitable EFI memory descriptor.
    if unsafe { ptr.as_ref() }.bitmap_coverage_end().is_none() {
        crate::println!("[EFI stub] warning: unaccepted memory table coverage overflows");
        return None;
    }

    // SAFETY: The complete table lies within a suitable EFI memory descriptor and the
    // bitmap layout has been validated above.
    Some(unsafe { ptr.as_mut() })
}

/// Creates a Linux-compatible unaccepted-memory bitmap table for `ranges`.
pub(crate) fn allocate_unaccepted_bitmap(
    ranges: &[UnacceptedRange],
) -> Option<&'static mut EfiUnacceptedMemory> {
    if ranges.is_empty() {
        return None;
    }

    let unit_size = EFI_UNACCEPTED_UNIT_SIZE;

    let mut min_addr = u64::MAX;
    let mut max_addr = 0u64;

    for range in ranges {
        debug_assert!(
            range.start < range.end,
            "invalid unaccepted range: {:#x} >= {:#x}",
            range.start,
            range.end
        );
        min_addr = min_addr.min(range.start);
        max_addr = max_addr.max(range.end);
    }

    let aligned_start = min_addr.align_down(unit_size);
    let aligned_end = max_addr.align_up(unit_size);

    let memory_range = aligned_end.checked_sub(aligned_start)?;
    let bitmap_bits = memory_range / unit_size;
    let bitmap_bytes = bitmap_bits.div_ceil(64) * 8;

    #[cfg(feature = "debug_print")]
    {
        uefi::println!("[EFI stub] Creating bitmap for unaccepted memory");
        uefi::println!(
            "[EFI stub] Range: {:#x} - {:#x}, Unit: {}KB, Bitmap: {} bytes",
            aligned_start,
            aligned_end,
            unit_size / 1024,
            bitmap_bytes
        );
    }

    let bitmap_bytes_usize = usize::try_from(bitmap_bytes).ok()?;
    let total_size = size_of::<EfiUnacceptedMemory>().checked_add(bitmap_bytes_usize)?;
    let pages = total_size.div_ceil(PAGE_SIZE as usize);

    // TDVF's first lazy-accept stage accepts all memory below 4 GiB. Keep the
    // bitmap allocation in that range so the ACPI_RECLAIM pages are already
    // accepted when they are returned.
    match uefi::boot::allocate_pages(
        AllocateType::MaxAddress(u32::MAX as u64),
        MemoryType::ACPI_RECLAIM,
        pages,
    ) {
        Ok(addr) => {
            // SAFETY: The returned pages are owned by us and large enough for header + bitmap.
            let table = unsafe { &mut *addr.as_ptr().cast::<EfiUnacceptedMemory>() };

            table
                .init_header(u32::try_from(unit_size).ok()?, aligned_start, bitmap_bytes)
                .ok()?;

            // SAFETY: `table` points to newly allocated pages large enough for
            // `EfiUnacceptedMemory` + bitmap bytes, and is uniquely borrowed here.
            unsafe { table.clear_bitmap() }.ok()?;

            Some(table)
        }
        Err(e) => {
            uefi::println!(
                "[EFI stub] error: failed to allocate bitmap memory: {:?}",
                e
            );
            None
        }
    }
}

/// Installs the unaccepted-memory bitmap table into EFI config tables.
pub(crate) fn install_unaccepted_bitmap(
    table: &EfiUnacceptedMemory,
) -> Result<(), InstallUnacceptedBitmapError> {
    let Some(st) = uefi::table::system_table_raw() else {
        uefi::println!("[EFI stub] error: system table is unavailable");
        return Err(InstallUnacceptedBitmapError::SystemTableUnavailable);
    };

    // SAFETY: `st` is provided by firmware and valid during boot services.
    let Some(boot_services) = (unsafe { (*st.as_ptr()).boot_services.as_ref() }) else {
        uefi::println!("[EFI stub] boot services are unavailable");
        return Err(InstallUnacceptedBitmapError::BootServicesUnavailable);
    };

    let install_config_table = boot_services.install_configuration_table;

    // SAFETY: Firmware boot services is valid at this phase; pointers passed follow EFI ABI.
    let status = unsafe {
        install_config_table(
            core::ptr::from_ref(&LINUX_EFI_UNACCEPTED_MEM_TABLE_GUID),
            core::ptr::from_ref(table).cast(),
        )
    };

    if !status.is_success() {
        uefi::println!(
            "[EFI stub] error: failed to install unaccepted memory table: {:?}",
            status
        );
        return Err(InstallUnacceptedBitmapError::InstallFailed);
    }

    #[cfg(feature = "debug_print")]
    {
        uefi::println!("[EFI stub] Unaccepted memory table installed successfully");
        uefi::println!("  Version: {}", table.version());
        uefi::println!("  Unit size: {}KB", table.unit_size_bytes() / 1024);
        uefi::println!("  Physical base: {:#x}", table.phys_base());
        uefi::println!("  Bitmap size: {} bytes", table.bitmap_size_bytes());
    }

    Ok(())
}

/// Accepts bitmap-marked unaccepted units that overlap `start..end`.
pub(crate) fn accept_bitmap_range(table: &mut EfiUnacceptedMemory, start: u64, end: u64) -> bool {
    if start >= end {
        return true;
    }

    // SAFETY: `table` is exclusively borrowed; method uses table-owned bitmap metadata.
    match unsafe { table.accept_range(start, end) } {
        Ok(()) => true,
        Err(err) => {
            crate::println!(
                "[EFI stub] error: failed to accept overlapping marked bitmap units: start={:#x}, end={:#x}, phys_base={:#x}, unit_size={}, bitmap_size={}, err={:?}",
                start,
                end,
                table.phys_base(),
                table.unit_size_bytes(),
                table.bitmap_size_bytes(),
                err
            );
            false
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum InstallUnacceptedBitmapError {
    SystemTableUnavailable,
    BootServicesUnavailable,
    InstallFailed,
}

/// Physical range that firmware marks as unaccepted.
#[derive(Debug, Clone, Copy)]
pub(crate) struct UnacceptedRange {
    pub start: u64,
    pub end: u64,
}

fn is_memory_range_valid<M: MemoryMap>(memory_map: &M, start: u64, size: usize) -> bool {
    let Ok(size) = u64::try_from(size) else {
        return false;
    };
    let Some(end) = start.checked_add(size) else {
        return false;
    };

    memory_map.entries().any(|entry| {
        let Some(entry_size) = entry.page_count.checked_mul(PAGE_SIZE) else {
            return false;
        };
        let Some(entry_end) = entry.phys_start.checked_add(entry_size) else {
            return false;
        };
        is_suitable_table_memory(entry.ty) && start >= entry.phys_start && end <= entry_end
    })
}

fn is_suitable_table_memory(memory_type: MemoryType) -> bool {
    matches!(
        memory_type,
        MemoryType::CONVENTIONAL
            | MemoryType::LOADER_DATA
            | MemoryType::BOOT_SERVICES_DATA
            | MemoryType::ACPI_RECLAIM
            | MemoryType::ACPI_NON_VOLATILE
            | MemoryType::RUNTIME_SERVICES_DATA
    )
}
