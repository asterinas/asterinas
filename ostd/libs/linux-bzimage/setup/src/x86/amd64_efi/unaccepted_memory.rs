// SPDX-License-Identifier: MPL-2.0

//! EFI-side bitmap management for TDX unaccepted memory.
//!
//! This module sets up TDX lazy acceptance and builds and installs the Linux-compatible
//! unaccepted-memory table.
//!
//! This path supports OVMF's TDVF implementation, which accepts all unaccepted
//! RAM below 4 GiB before handing control to the EFI stub. In particular,
//! td-shim uses a different lazy-accept contract: it accepts an initial memory budget and
//! passes the remaining memory to the payload in an unaccepted-memory bitmap.

extern crate alloc;

use alloc::vec::Vec;
use core::ops::Range;

use tdx_guest::{
    is_tdx_guest_early,
    unaccepted_memory::{EfiUnacceptedMemory, LINUX_EFI_UNACCEPTED_MEM_TABLE_GUID},
};
use uefi::{
    boot::{AllocateType, MemoryType},
    mem::memory_map::MemoryMap,
};

use crate::x86::amd64_efi::efi::PAGE_SIZE;

// This boundary comes from the supported TDVF implementation.
const ACCEPTED_LOW_MEMORY_END: u64 = 1 << 32;

/// Sets up the unaccepted-memory bitmap consumed by the kernel.
pub(super) fn setup_lazy_accept() {
    if !is_tdx_guest_early() {
        return;
    }

    let pre_exit_memory_map = uefi::boot::memory_map(MemoryType::LOADER_DATA)
        .expect("[EFI stub] failed to fetch pre-exit memory map for unaccepted bitmap setup");

    let unaccepted_ranges: Vec<_> = pre_exit_memory_map
        .entries()
        .filter(|entry| entry.ty == MemoryType::UNACCEPTED)
        .map(|entry| {
            let size = entry.page_count * PAGE_SIZE;
            let end = entry.phys_start + size;
            assert!(
                entry.phys_start >= ACCEPTED_LOW_MEMORY_END,
                "[EFI stub] unsupported unaccepted memory below 4 GiB: [{:#x}, {:#x}); firmware must accept all low memory",
                entry.phys_start,
                end
            );

            entry.phys_start..end
        })
        .collect();

    if unaccepted_ranges.is_empty() {
        return;
    }

    uefi::println!("[EFI stub] Installing EFI-stub unaccepted memory table");
    let table = allocate_unaccepted_bitmap(&unaccepted_ranges)
        .unwrap_or_else(|| panic!("[EFI stub] failed to allocate unaccepted bitmap table"));
    install_unaccepted_bitmap(table).unwrap_or_else(|err| {
        panic!(
            "[EFI stub] failed to install unaccepted bitmap table: {:?}",
            err
        )
    });
}

/// Creates a Linux-compatible unaccepted-memory bitmap table for `ranges`.
fn allocate_unaccepted_bitmap(ranges: &[Range<u64>]) -> Option<&'static mut EfiUnacceptedMemory> {
    let required_size = EfiUnacceptedMemory::required_size(ranges).ok()??;
    let pages = required_size.div_ceil(PAGE_SIZE as usize);

    // TDVF's first lazy-accept stage accepts all memory below 4 GiB. Keep the
    // bitmap allocation in that range so the ACPI_RECLAIM pages are already
    // accepted when they are returned.
    match uefi::boot::allocate_pages(
        AllocateType::MaxAddress(u32::MAX as u64),
        MemoryType::ACPI_RECLAIM,
        pages,
    ) {
        Ok(addr) => {
            let allocation_size = pages * PAGE_SIZE as usize;
            // SAFETY: The returned pages are uniquely owned, large enough for
            // the table, and remain allocated after EFI boot services exit.
            unsafe { EfiUnacceptedMemory::new(addr, allocation_size, ranges) }.ok()
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
fn install_unaccepted_bitmap(
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

#[derive(Debug, Clone, Copy)]
enum InstallUnacceptedBitmapError {
    SystemTableUnavailable,
    BootServicesUnavailable,
    InstallFailed,
}
