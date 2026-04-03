// SPDX-License-Identifier: MPL-2.0

//! EFI-side bitmap management for TDX unaccepted memory.
//!
//! This module builds and installs the Linux-compatible unaccepted-memory table,
//! marks deferred-accept ranges in the bitmap, and eagerly accepts only the
//! ranges that must be mapped immediately.

use align_ext::AlignExt;
use tdx_guest::unaccepted_memory::{
    BitmapSegmentLocks, EFI_UNACCEPTED_UNIT_SIZE, EfiUnacceptedMemory,
    LINUX_EFI_UNACCEPTED_MEM_TABLE_GUID, LINUX_EFI_UNACCEPTED_MEM_TABLE_VERSION,
};
use uefi::table::boot::{AllocateType, MemoryType};

pub(crate) fn find_unaccepted_table() -> Option<&'static mut EfiUnacceptedMemory> {
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
    for entry in entries {
        if entry.vendor_guid == LINUX_EFI_UNACCEPTED_MEM_TABLE_GUID {
            // SAFETY: Matching config-table entry points to a valid installed table.
            let table = unsafe { &mut *entry.vendor_table.cast::<EfiUnacceptedMemory>() };

            let version = table.version();
            if version != LINUX_EFI_UNACCEPTED_MEM_TABLE_VERSION {
                crate::println!(
                    "[EFI stub] warning: unknown unaccepted memory table version: {}",
                    version
                );
                return None;
            }

            return Some(table);
        }
    }

    None
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
        min_addr = min_addr.min(range.start);
        max_addr = max_addr.max(range.end);
    }

    let aligned_start = min_addr.align_down(unit_size);
    let aligned_end = max_addr.align_up(unit_size);

    let memory_range = aligned_end - aligned_start;
    let bitmap_bits = memory_range / unit_size;
    let bitmap_bytes = bitmap_bits.div_ceil(8);

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
    let total_size = core::mem::size_of::<EfiUnacceptedMemory>().checked_add(bitmap_bytes_usize)?;
    let pages = total_size.div_ceil(4096);

    match uefi::boot::allocate_pages(AllocateType::AnyPages, MemoryType::ACPI_RECLAIM, pages) {
        Ok(addr) => {
            // SAFETY: The returned pages are owned by us and large enough for header + bitmap.
            let table = unsafe { &mut *addr.as_ptr().cast::<EfiUnacceptedMemory>() };

            table
                .init_header(u32::try_from(unit_size).ok()?, aligned_start, bitmap_bytes)
                .ok()?;

            // SAFETY: `table` points to newly allocated pages large enough for
            // `EfiUnacceptedMemory` + bitmap bytes, and is uniquely borrowed here.
            let bitmap = unsafe { table.as_bitmap_slice_mut() };
            bitmap.fill(0);

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
pub(crate) fn accept_bitmap_range(
    table: &mut EfiUnacceptedMemory,
    start: u64,
    end: u64,
    locks: &BitmapSegmentLocks<'_>,
) -> bool {
    if start >= end {
        return true;
    }

    // SAFETY: `table` is exclusively borrowed; method uses table-owned bitmap metadata.
    match unsafe { table.accept_range(start, end, locks) } {
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
