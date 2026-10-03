// SPDX-License-Identifier: MPL-2.0

//! EFI-side management of TDX unaccepted memory.
//!
//! The EFI stub records unaccepted memory ranges from the final UEFI memory map
//! in a bitmap and passes the table to the kernel through the boot parameters.
//! It reserves the bitmap storage before exiting boot services, then registers
//! the ranges after the final memory map is available.
//!
//! Asterinas uses a fixed kernel load address below 4 GiB, so the EFI stub does
//! not need to accept memory for an arbitrary kernel destination. When ranges
//! are registered, small regions and unaligned edges are accepted immediately;
//! the remaining aligned regions are left for the kernel to accept later.

use core::{ops::Range, ptr::NonNull};

use linux_boot_params::{BootParams, EfiInfo};
use tdx_guest::unaccepted_memory::{EfiUnacceptedMemory, EfiUnacceptedMemoryBuilder};
use uefi::{
    boot::AllocateType,
    mem::memory_map::{MemoryMap, MemoryType},
};

use crate::x86::amd64_efi::{alloc, efi};

/// Creates a builder for the unaccepted memory bitmap.
///
/// This method is called before exiting the EFI boot services to make memory allocations. Afterwards,
/// the builder can build the final bitmap without allocations. See [`finish_unaccepted_memory`].
pub(super) fn prepare_unaccepted_memory(
    boot_params: &mut BootParams,
) -> Option<EfiUnacceptedMemoryBuilder> {
    if !tdx_guest::is_tdx_guest_early() {
        return None;
    }

    boot_params.efi_info.efi_loader_signature = EfiInfo::ASTERINAS_LOADER_SIGNATURE;
    boot_params.efi_info.efi_systab = 0;
    boot_params.efi_info.efi_systab_hi = 0;

    // Compute a single range that covers all unaccepted memory.
    let coverage = {
        let pre_exit_memory_map = uefi::boot::memory_map(MemoryType::LOADER_DATA) 
            .expect("[EFI stub] failed to fetch pre-exit memory map for unaccepted bitmap setup");
        let mut ranges = iter_unaccepted_memory(&pre_exit_memory_map);

        let first_range = ranges.next()?;
        ranges.fold(first_range, |coverage, range| {
            coverage.start.min(range.start)..coverage.end.max(range.end)
        })
    };

    // Allocate pages for the unaccepted memory bitmap.
    let builder = {
        let required_size = EfiUnacceptedMemory::required_size_for_range(coverage.clone())
            .expect("[EFI stub] invalid unaccepted memory bounds");

        let allocation = alloc::alloc_pages(AllocateType::MaxAddress(u32::MAX as u64), required_size);
        let allocation_ptr = NonNull::new(allocation.as_mut_ptr().cast()).unwrap();
        // SAFETY: `allocation` is writable, page-aligned, and sized for `coverage`.
        // After this call, the allocation is accessed only through the builder and
        // the resulting table, and the EFI stub never frees it.
        unsafe { EfiUnacceptedMemory::builder(allocation_ptr, allocation.len(), coverage) }
            .expect("[EFI stub] failed to initialize unaccepted memory builder")
    };

    uefi::println!("[EFI stub] Passing unaccepted memory bitmap to the kernel");

    Some(builder)
}

/// Builds the unaccepted memory bitmap according to `memory_map` and publishes the table address to
/// the kernel.
///
/// This method may accept memory, but it will not update `memory_map`.
///
/// # Safety
///
/// `memory_region` must accurately describe the regions of unaccepted memory.
pub(super) fn finish_unaccepted_memory<M: MemoryMap>(
    boot_params: &mut BootParams,
    builder: Option<EfiUnacceptedMemoryBuilder>,
    memory_map: &M,
) {
    let Some(mut builder) = builder else {
        return;
    };

    for range in iter_unaccepted_memory(memory_map) {
        // SAFETY: The range comes directly from firmware UEFI memory map unaccepted entries.
        unsafe { builder.register_range(range.start, range.end) }
            .expect("[EFI stub] failed to register unaccepted memory range");
    }

    let table = builder.build();
    let table_addr = core::ptr::from_ref(table.as_ref().get_ref()).addr();
    let table_addr = table_addr
        .try_into()
        .expect("[EFI stub] unaccepted memory table is above 4 GiB");
    boot_params.efi_info.efi_systab = table_addr;
}

fn iter_unaccepted_memory<M: MemoryMap>(memory_map: &M) -> impl Iterator<Item = Range<u64>> + '_ {
    memory_map.entries().filter_map(|entry| {
        if entry.ty != MemoryType::UNACCEPTED {
            return None;
        }

        let size = entry.page_count * efi::PAGE_SIZE;
        let end = entry.phys_start + size;
        Some(entry.phys_start..end)
    })
}
