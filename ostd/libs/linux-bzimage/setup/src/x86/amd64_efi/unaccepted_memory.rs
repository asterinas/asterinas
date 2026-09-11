// SPDX-License-Identifier: MPL-2.0

//! EFI-side bitmap management for TDX unaccepted memory.
//!
//! This module scans the UEFI memory map for `EFI_UNACCEPTED_MEMORY` regions, builds
//! the Linux-compatible unaccepted-memory table, and passes it to the kernel through
//! the boot parameters.
//!
//! ### EFI boot strategy
//!
//! Linux's EFI stub supports physical KASLR, so the decompressed kernel may be
//! placed at an arbitrary physical address. The initrd and boot structures can
//! likewise be allocated in unaccepted memory. The stub therefore accepts memory
//! as needed while allocating and decompressing these objects.
//!
//! Asterinas instead uses a fixed kernel load address below 4 GiB, as defined by
//! its boot layout. The EFI stub consequently does not need allocation-time
//! acceptance: it records the unaccepted ranges in the table and passes it to the
//! kernel.

//! Firmware Contract
//!
//! This path relies on OVMF's standard TDVF contract, which accepts all RAM below 4 GiB
//! before invoking the EFI stub and marks remaining RAM above 4 GiB as unaccepted.
//! (In contrast, alternative firmwares like td-shim accept only a minimal budget). All
//! unaccepted regions below 4 GiB are treated as unsupported by this loader.

use core::ptr::NonNull;

use tdx_guest::{is_tdx_guest_early, unaccepted_memory::EfiUnacceptedMemory};
use uefi::{
    boot::AllocateType,
    mem::memory_map::{MemoryMap, MemoryType},
};

use crate::x86::amd64_efi::{alloc::alloc_pages, efi::PAGE_SIZE};

/// Sets up the unaccepted-memory bitmap consumed by the kernel.
pub(super) fn setup_unaccepted_memory(boot_params: &mut linux_boot_params::BootParams) {
    if !is_tdx_guest_early() {
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

        let size = entry.page_count * PAGE_SIZE;
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
    let allocation = alloc_pages(AllocateType::MaxAddress(u32::MAX as u64), required_size);
    let allocation_ptr = NonNull::new(allocation.as_mut_ptr().cast()).unwrap();

    // Start building the table covering [min_addr, max_addr).
    let mut builder = unsafe {
        EfiUnacceptedMemory::builder(allocation_ptr, allocation.len(), coverage)
    }
    .expect("[EFI stub] failed to initialize unaccepted memory builder");

    for entry in pre_exit_memory_map.entries() {
        if entry.ty != MemoryType::UNACCEPTED {
            continue;
        }

        let size = entry.page_count * PAGE_SIZE;
        let end = entry.phys_start + size;
        // SAFETY: The range comes directly from firmware UEFI memory map unaccepted entries.
        unsafe { builder.register_range(entry.phys_start, end) }
            .expect("[EFI stub] failed to register unaccepted memory range");
    }

    let mut table = builder.build();
    boot_params.unaccepted_memory =
        unsafe { table.as_mut().get_unchecked_mut() } as *mut EfiUnacceptedMemory as u64;
}
