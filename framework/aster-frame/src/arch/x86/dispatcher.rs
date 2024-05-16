// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;

use align_ext::AlignExt;

use crate::{
    arch::device::io_port::IO_PORT_MAX,
    boot::memory_region::{MemoryRegion, MemoryRegionType},
    device::dispatcher::{io_mem::IoMemDispatcher, io_port::IoPortDispatcher},
};

/// This function will initialize the allocatable MMIO area based on the x86-64 memory distribution map.
///
/// In x86-64, the available physical memory area is divided into two regions below 32 bits (Low memory)
/// and above (High memory). Where the area from the top of low memory to 0xffff_ffff, after the top of
/// high memory, is the available MMIO area.
///
pub(crate) fn init_io_mem_dispatcher(mut regions: Vec<MemoryRegion>, dispatcher: &IoMemDispatcher) {
    if regions.is_empty() {
        panic!("Memory Regions is empty.")
    }
    regions.sort_by_key(|a| a.base());
    regions.reverse();

    let mut is_tohm_found = false;
    const HIGH_MMIO_SIZE: usize = 0x100_0000;
    for region in regions {
        if !is_tohm_found && (region.base() + region.len()) > u32::MAX as usize {
            // Find the TOHM (Top of High Memory) and initialize High MMIO region.
            // Here, using 16MB (0x100_0000) as the size of High MMIO region.
            // TODO: Update the High MMIO region in runtime.
            if region.typ() == MemoryRegionType::Usable {
                is_tohm_found = true;
                // Align start address to 0x8_0000_0000
                let mmio_start_addr = (region.base() + region.len()).align_up(0x8_0000_0000);
                // SAFETY: The range is guaranteed not to access physical memory.
                unsafe {
                    dispatcher.add_range(mmio_start_addr..(mmio_start_addr + HIGH_MMIO_SIZE));
                }
            }
        } else if (region.base() + region.len()) < u32::MAX as usize {
            // Find the TOLM (Top of Low Memory) and initialize Low MMIO region (TOLM ~ 0xffff_ffff).
            if region.typ() == MemoryRegionType::Usable {
                // Align start address to 0x1000_0000
                let mmio_start_addr = (region.base() + region.len()).align_up(0x1000_0000);
                // SAFETY: The range is guaranteed not to access physical memory.
                unsafe {
                    dispatcher.add_range(mmio_start_addr..0x1_0000_0000);
                }
                break;
            }
        }
    }
    if !is_tohm_found {
        let mmio_start_addr = 0x8_0000_0000;
        // SAFETY: The range is guaranteed not to access physical memory.
        unsafe {
            dispatcher.add_range(mmio_start_addr..(mmio_start_addr + HIGH_MMIO_SIZE));
        }
    }
}

/// This function will initialize the allocatable MMIO area based on the x86-64 memory distribution map.
pub(crate) fn init_io_port_dispatcher() -> IoPortDispatcher {
    IoPortDispatcher::new(IO_PORT_MAX)
}
