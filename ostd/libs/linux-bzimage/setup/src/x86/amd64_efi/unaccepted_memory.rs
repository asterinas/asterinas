// SPDX-License-Identifier: MPL-2.0

//! EFI-side bitmap management for TDX unaccepted memory.
//!
//! This module scans the UEFI memory map for `EFI_UNACCEPTED_MEMORY` regions, builds
//! the Linux-compatible unaccepted-memory configuration table, and installs it into
//! the EFI system configuration table for the kernel.
//!
//! ### EFI boot strategy
//!
//! Linux's EFI stub supports physical KASLR, so the decompressed kernel may be
//! placed at an arbitrary physical address. The initrd and boot structures can
//! likewise be allocated in unaccepted memory. The stub therefore accepts memory
//! as needed while allocating and decompressing these objects.
//!
//! Asterinas instead uses a fixed kernel load address below 4 GiB, as defined by
//! its boot layout. Independently, the supported TDVF contract guarantees that RAM
//! below 4 GiB is already accepted. The EFI stub consequently does not need
//! allocation-time acceptance: it records the unaccepted ranges in the standard EFI
//! table and passes them to the kernel.
//!
//! ### Firmware Contract
//!
//! This path relies on OVMF's standard TDVF contract, which accepts all RAM below 4 GiB
//! before invoking the EFI stub and marks remaining RAM above 4 GiB as unaccepted.
//! (In contrast, alternative firmwares like td-shim accept only a minimal budget). All
//! unaccepted regions below 4 GiB are treated as unsupported by this loader.

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

// The supported TDVF implementation accepts all RAM below 4 GiB.
const ACCEPTED_LOW_MEMORY_END: u64 = 0x1_0000_0000;

/// Sets up the unaccepted-memory bitmap consumed by the kernel.
pub(super) fn setup_unaccepted_memory() {
    if !is_tdx_guest_early() {
        return;
    }

    let pre_exit_memory_map = uefi::boot::memory_map(MemoryType::LOADER_DATA)
        .expect("[EFI stub] failed to fetch pre-exit memory map for unaccepted bitmap setup");

    let mut unaccepted_ranges = Vec::new();
    for entry in pre_exit_memory_map.entries() {
        if entry.ty != MemoryType::UNACCEPTED {
            continue;
        }

        let size = entry.page_count * PAGE_SIZE;
        let end = entry.phys_start + size;
        assert!(
            entry.phys_start >= ACCEPTED_LOW_MEMORY_END,
            "[EFI stub] unsupported unaccepted memory below 4 GiB: [{:#x}, {:#x}); firmware must accept all low memory",
            entry.phys_start,
            end
        );

        unaccepted_ranges.push(entry.phys_start..end);
    }

    if unaccepted_ranges.is_empty() {
        return;
    }

    uefi::println!("[EFI stub] Installing EFI-stub unaccepted memory table");
    let table = allocate_unaccepted_bitmap(&unaccepted_ranges);
    install_unaccepted_bitmap(table);
}

/// Creates a Linux-compatible unaccepted-memory bitmap table for `ranges`.
fn allocate_unaccepted_bitmap(ranges: &[Range<u64>]) -> &'static mut EfiUnacceptedMemory {
    let required_size = EfiUnacceptedMemory::required_size(ranges)
        .expect("[EFI stub] invalid unaccepted memory ranges");
    let pages = required_size.div_ceil(PAGE_SIZE as usize);

    // Allocate the bitmap below 4 GiB, where TDVF has already accepted the memory.
    // Keep it marked as ACPI reclaimable until the kernel consumes the table.
    let addr = uefi::boot::allocate_pages(
        AllocateType::MaxAddress(u32::MAX as u64),
        MemoryType::ACPI_RECLAIM,
        pages,
    )
    .expect("[EFI stub] failed to allocate unaccepted bitmap pages");
    let allocation_size = pages * PAGE_SIZE as usize;

    // SAFETY: The returned pages are uniquely owned, large enough for
    // the table, and remain allocated after EFI boot services exit.
    unsafe { EfiUnacceptedMemory::new(addr, allocation_size, ranges) }
        .expect("[EFI stub] failed to initialize unaccepted memory table")
}

/// Installs the unaccepted-memory bitmap table into EFI config tables.
fn install_unaccepted_bitmap(table: &EfiUnacceptedMemory) {
    let system_table = uefi::table::system_table_raw()
        .expect("[EFI stub] system table is unavailable");

    // SAFETY: `system_table` is provided by firmware and valid during boot services.
    let boot_services = unsafe { (*system_table.as_ptr()).boot_services.as_ref() }
        .expect("[EFI stub] boot services are unavailable");

    let install_config_table = boot_services.install_configuration_table;

    // SAFETY: Firmware boot services is valid at this phase; pointers passed follow EFI ABI.
    let status = unsafe {
        install_config_table(
            core::ptr::from_ref(&LINUX_EFI_UNACCEPTED_MEM_TABLE_GUID),
            core::ptr::from_ref(table).cast(),
        )
    };

    assert!(
        status.is_success(),
        "[EFI stub] failed to install unaccepted memory configuration table: {status:?}"
    );

    #[cfg(feature = "debug_print")]
    {
        uefi::println!("[EFI stub] Unaccepted memory table installed successfully");
        uefi::println!("  Version: {}", table.version());
        uefi::println!("  Unit size: {}KB", table.unit_size_bytes() / 1024);
        uefi::println!("  Physical base: {:#x}", table.phys_base());
        uefi::println!("  Bitmap size: {} bytes", table.bitmap_size_bytes());
    }
}
