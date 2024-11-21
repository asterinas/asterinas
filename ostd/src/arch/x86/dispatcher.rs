// SPDX-License-Identifier: MPL-2.0

use alloc::{borrow::ToOwned, vec::Vec};

use align_ext::AlignExt;

use crate::{
    boot::memory_region::MemoryRegionType, device::dispatcher::io_mem::IoMemDispatcherBuilder,
};

/// Initializes the allocatable MMIO area based on the x86-64 memory distribution map.
///
/// In x86-64, the available physical memory area is divided into two regions below 32 bits (Low memory)
/// and above (High memory). The area from the top of low memory to 0xffff_ffff and the area after the
/// top of high memory are available MMIO areas.
pub(super) fn construct_io_mem_dispatcher_builder() -> IoMemDispatcherBuilder {
    // TODO: Add MMIO regions below 1MB (e.g., VGA framebuffer).
    let reversed_regions = {
        let mut regions = crate::boot::memory_regions().to_owned();
        assert!(!regions.is_empty());
        regions.sort_by_key(|a| a.base());
        regions.reverse();
        regions
    };

    let mut ranges = Vec::with_capacity(2);
    let mut is_tohm_found = false;

    const LOW_MMIO_TOP: usize = 0x1_0000_0000;
    const LOW_MMIO_ALIGN: usize = 0x1000_0000;
    const HIGH_MMIO_TOP: usize = 0x8000_0000_0000;
    const HIGH_MMIO_ALIGN: usize = 0x1_0000_0000;

    for region in reversed_regions {
        if region.typ() != MemoryRegionType::Usable {
            continue;
        }

        if !is_tohm_found && region.base() > u32::MAX as usize {
            // Find the TOHM (Top of High Memory) and initialize High MMIO region.
            // Here, using HIGH_MMIO_TOP as the top of High MMIO region.
            //
            // TODO: Update the High MMIO region in runtime.
            is_tohm_found = true;
            // Align start address to HIGH_MMIO_ALIGN
            let mmio_start_addr = (region.base() + region.len()).align_up(HIGH_MMIO_ALIGN);
            assert!(mmio_start_addr < HIGH_MMIO_TOP);
            ranges.push(mmio_start_addr..HIGH_MMIO_TOP);
        } else if region.base() < u32::MAX as usize {
            // Find the TOLM (Top of Low Memory) and initialize Low MMIO region (TOLM ~ LOW_MMIO_TOP).
            // Align start address to LOW_MMIO_ALIGN
            let mmio_start_addr = (region.base() + region.len()).align_up(LOW_MMIO_ALIGN);
            assert!(mmio_start_addr < LOW_MMIO_TOP);
            ranges.push(mmio_start_addr..LOW_MMIO_TOP);
            break;
        }
    }

    if !is_tohm_found {
        ranges.push(HIGH_MMIO_ALIGN..HIGH_MMIO_TOP);
    }

    // SAFETY: The range is guaranteed not to access physical memory.
    unsafe { IoMemDispatcherBuilder::new(ranges) }
}
