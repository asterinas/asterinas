// SPDX-License-Identifier: MPL-2.0

//! EFI-side bitmap management for TDX unaccepted memory.

use core::{ops::Range, ptr::NonNull};

use linux_boot_params::{BootParams, EfiInfo};
use tdx_guest::unaccepted_memory::{EfiUnacceptedMemory, EfiUnacceptedMemoryBuilder};
use uefi::{
    boot::AllocateType,
    mem::memory_map::{MemoryMap, MemoryType},
};

use crate::x86::amd64_efi::{alloc, efi};

/// Allocates below-4-GiB table storage before exiting EFI boot services.
pub(super) fn prepare_unaccepted_memory(
    boot_params: &mut BootParams,
) -> Option<EfiUnacceptedMemoryBuilder> {
    if !tdx_guest::is_tdx_guest_early() {
        return None;
    }

    boot_params.efi_info.efi_loader_signature = EfiInfo::ASTERINAS_LOADER_SIGNATURE;
    boot_params.efi_info.efi_systab = 0;
    boot_params.efi_info.efi_systab_hi = 0;

    let pre_exit_memory_map = uefi::boot::memory_map(MemoryType::LOADER_DATA)
        .expect("[EFI stub] failed to fetch pre-exit memory map for unaccepted bitmap setup");

    let coverage = unaccepted_memory_coverage(&pre_exit_memory_map)?;

    uefi::println!("[EFI stub] Passing unaccepted memory bitmap to the kernel");

    let required_size = EfiUnacceptedMemory::required_size_for_range(coverage.clone())
        .expect("[EFI stub] invalid unaccepted memory bounds");
    let allocation = alloc::alloc_pages(AllocateType::MaxAddress(u32::MAX as u64), required_size);
    let allocation_ptr = NonNull::new(allocation.as_mut_ptr().cast()).unwrap();

    // SAFETY: `allocation` is writable, page-aligned, and sized for `coverage`.
    // After this call, the allocation is accessed only through the builder and
    // the resulting table, and the EFI stub never frees it.
    Some(
        unsafe { EfiUnacceptedMemory::builder(allocation_ptr, allocation.len(), coverage) }
            .expect("[EFI stub] failed to initialize unaccepted memory builder"),
    )
}

/// Registers final-map unaccepted ranges and publishes the table address to the kernel.
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

fn unaccepted_memory_coverage<M: MemoryMap>(memory_map: &M) -> Option<Range<u64>> {
    let mut ranges = iter_unaccepted_memory(memory_map);
    let first_range = ranges.next()?;

    Some(ranges.fold(first_range, |coverage, range| {
        coverage.start.min(range.start)..coverage.end.max(range.end)
    }))
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
