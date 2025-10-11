// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;

use crate::{boot::memory_region::MemoryRegionType, io::IoMemAllocatorBuilder};

/// Initializes the allocatable MMIO area based on the LoongArch memory
/// distribution map.
///
/// Here we consider all the holes (filtering usable RAM) in the physical
/// address space as MMIO regions.
///
/// # Safety
///
/// 1. This function must be called only once in the boot context of the
///    bootstrapping processor.
/// 2. This function must be called after the kernel page table is activated on
///    the bootstrapping processor.
pub(super) unsafe fn construct_io_mem_allocator_builder() -> IoMemAllocatorBuilder {
    let regions = &crate::boot::EARLY_INFO.get().unwrap().memory_regions;
    let reserved_filter = regions.iter().filter(|r| {
        r.typ() != MemoryRegionType::Unknown
            && r.typ() != MemoryRegionType::Reserved
            && r.typ() != MemoryRegionType::Framebuffer
    });

    let mut ranges = Vec::new();

    let mut current_address = 0;
    for region in reserved_filter {
        if current_address < region.base() {
            ranges.push(current_address..region.base());
        }
        current_address = region.end();
    }
    if current_address < usize::MAX {
        ranges.push(current_address..usize::MAX);
    }

    // SAFETY:
    // 1. The caller ensures that the kernel page table is already activated.
    // 2. The range is guaranteed not to access physical memory.
    unsafe { IoMemAllocatorBuilder::new(ranges) }
}
