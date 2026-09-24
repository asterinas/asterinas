// SPDX-License-Identifier: MPL-2.0

//! EFI-side bitmap management for TDX unaccepted memory.
//!
//! This module scans the UEFI memory map for `EFI_UNACCEPTED_MEMORY` regions, builds
//! the Linux-compatible unaccepted-memory table, and passes it to the kernel through
//! the boot parameters.
//!
//! ### EFI Boot Strategy
//!
//! Linux's EFI stub supports physical KASLR, so the decompressed kernel may be
//! placed at an arbitrary physical address. The initrd and boot structures can
//! likewise be allocated in unaccepted memory. The stub therefore accepts memory
//! as needed while allocating and decompressing these objects.
//!
//! Asterinas instead uses a fixed kernel load address below 4 GiB, as defined by
//! its boot layout. The EFI stub consequently does not need allocation-time
//! acceptance: it records the unaccepted ranges in the table and passes it to the
//! kernel. Registering ranges eagerly accepts regions smaller than 4 MiB and
//! unaligned edges via TDCALL, while the remaining aligned portions are recorded
//! in the bitmap for subsequent acceptance in the kernel.
//!
//! ### Firmware Assumptions
//!
//! This path is designed around standard TDVF behavior, where memory needed for
//! early boot structures below 4 GiB is already accepted by the firmware. Any
//! unaccepted memory regions identified in the UEFI memory map are recorded in
//! the bitmap and left for the kernel to accept.

use core::ptr::NonNull;

use linux_boot_params::EfiInfo;
use tdx_guest::unaccepted_memory::EfiUnacceptedMemory;
use uefi::{
    boot::AllocateType,
    mem::memory_map::{MemoryMap, MemoryType},
};

use crate::x86::amd64_efi::{alloc, efi};

/// Sets up the unaccepted-memory bitmap consumed by the kernel.
pub(super) fn setup_unaccepted_memory(boot_params: &mut linux_boot_params::BootParams) {
    boot_params.efi_info.efi_loader_signature = EfiInfo::ASTERINAS_LOADER_SIGNATURE;
    boot_params.efi_info.efi_systab = 0;
    boot_params.efi_info.efi_systab_hi = 0;

    if !tdx_guest::is_tdx_guest_early() {
        return;
    }

    // The TDVF contract guarantees that allocating memory below 4 GiB cannot
    // change the set of unaccepted ranges (which are all above 4 GiB).
    let pre_exit_memory_map = uefi::boot::memory_map(MemoryType::LOADER_DATA)
        .expect("[EFI stub] failed to fetch pre-exit memory map for unaccepted bitmap setup");

    let mut min_addr = u64::MAX;
    let mut max_addr = 0u64;

    for entry in pre_exit_memory_map.entries() {
        if entry.ty != MemoryType::UNACCEPTED {
            continue;
        }

        let size = entry.page_count * efi::PAGE_SIZE;
        let end = entry.phys_start + size;
        min_addr = min_addr.min(entry.phys_start);
        max_addr = max_addr.max(end);
    }

    if min_addr >= max_addr {
        return;
    }

    uefi::println!("[EFI stub] Passing unaccepted memory bitmap to the kernel");

    let coverage = min_addr..max_addr;

    // Allocate the table memory below 4 GiB.
    let required_size = EfiUnacceptedMemory::required_size_for_range(coverage.clone())
        .expect("[EFI stub] invalid unaccepted memory bounds");
    let allocation = alloc::alloc_pages(AllocateType::MaxAddress(u32::MAX as u64), required_size);
    let allocation_ptr = NonNull::new(allocation.as_mut_ptr().cast()).unwrap();

    // SAFETY: `allocation` is writable, page-aligned, and sized for `coverage`.
    // After this call, the allocation is accessed only through the builder and
    // the resulting table, and the EFI stub never frees it.
    // The kernel reserves the header and bitmap before frame allocation,
    // preserving the table storage across the EFI handoff.
    let mut builder = unsafe {
        EfiUnacceptedMemory::builder(allocation_ptr, allocation.len(), coverage)
    }
    .expect("[EFI stub] failed to initialize unaccepted memory builder");

    for entry in pre_exit_memory_map.entries() {
        if entry.ty != MemoryType::UNACCEPTED {
            continue;
        }

        let size = entry.page_count * efi::PAGE_SIZE;
        let end = entry.phys_start + size;
        // SAFETY: The range comes directly from firmware UEFI memory map unaccepted entries.
        unsafe { builder.register_range(entry.phys_start, end) }
            .expect("[EFI stub] failed to register unaccepted memory range");
    }

    let table = builder.build();
    let table_addr = core::ptr::from_ref(table.as_ref().get_ref()).addr();
    boot_params.efi_info.efi_systab = table_addr
        .try_into()
        .expect("[EFI stub] unaccepted memory table is above 4 GiB");
}
