// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;

use align_ext::AlignExt;

use crate::{boot::memory_region::MemoryRegionType, io::IoMemAllocatorBuilder};

/// Initializes the allocatable MMIO area based on the x86-64 memory distribution map.
///
/// In x86-64, the available physical memory area is divided into two regions below 32 bits (Low memory)
/// and above (High memory). The area from the top of low memory to 0xffff_ffff and the area after the
/// top of high memory are available MMIO areas.
///
/// This memory layout is documented in Intel's datasheets. The MMIO area is defined by specific
/// registers that are configured by the BIOS and locked down, preventing the OS from reconfiguring
/// them. For the details, one can read the "Processor Configuration Register Definitions and
/// Address Ranges" section in the "10th Generation Intel(R) Processor Families" datasheet
/// (<https://www.intel.com/content/dam/www/public/us/en/documents/datasheets/10th-gen-core-families-datasheet-vol-2-datasheet.pdf>).
/// However, note that these specifics may differ between CPU generations and those manufactured by
/// other vendors.
pub(super) fn construct_io_mem_allocator_builder() -> IoMemAllocatorBuilder {
    // TODO: Add MMIO regions below 1MB (e.g., VGA framebuffer).
    let regions = &crate::boot::EARLY_INFO.get().unwrap().memory_regions;
    let mut ranges = Vec::with_capacity(2);

    let reserved_filter = regions.iter().filter(|r| {
        r.typ() != MemoryRegionType::Unknown
            && r.typ() != MemoryRegionType::Reserved
            && r.typ() != MemoryRegionType::Framebuffer
    });

    // Find the TOLM (Top of Low Memory) and initialize Low MMIO region (TOLM ~ LOW_MMIO_TOP).
    // Align start address to LOW_MMIO_ALIGN (according to Intel's datasheets, the lower 20 bits
    // are zeroed).
    const LOW_MMIO_TOP: usize = 0x1_0000_0000; // 4 GiB, 32 bits
    const LOW_MMIO_ALIGN: usize = 0x10_0000; // 1 MiB, 20 bits
    let (lower_half_base, lower_half_len) = reserved_filter
        .clone()
        .filter(|r| r.base() < LOW_MMIO_TOP)
        .max_by(|a, b| a.base().cmp(&b.base()))
        .map(|reg| (reg.base(), reg.len()))
        .unwrap();

    let mmio_start_addr = (lower_half_base + lower_half_len).align_up(LOW_MMIO_ALIGN);
    assert!(mmio_start_addr < LOW_MMIO_TOP);
    ranges.push(mmio_start_addr..LOW_MMIO_TOP);

    // Find the TOHM (Top of High Memory) and initialize High MMIO region (TOHM ~ HIGH_MMIO_TOP).
    // Align start address to HIGH_MMIO_ALIGN (according to Intel's datasheets, the lower 20 bits
    // are zeroed).
    //
    // TODO: Use the CPUID instruction to determine the maximum number of bits in physical
    // addresses. We use 52 bits here, which is the architectural limit for physical addresses.
    const HIGH_MMIO_TOP: usize = 0x10_0000_0000_0000; // 4 PiB, 52 bits
    const HIGH_MMIO_ALIGN: usize = 0x10_0000; // 1 MiB, 20 bits
    let (upper_half_base, upper_half_len) = reserved_filter
        .filter(|r| r.base() >= LOW_MMIO_TOP)
        .max_by(|a, b| a.base().cmp(&b.base()))
        .map(|reg| (reg.base(), reg.len()))
        .unwrap_or((LOW_MMIO_TOP, 0));

    let mmio_start_addr = (upper_half_base + upper_half_len).align_up(HIGH_MMIO_ALIGN);
    assert!(mmio_start_addr < HIGH_MMIO_TOP);
    ranges.push(mmio_start_addr..HIGH_MMIO_TOP);

    // SAFETY: The range is guaranteed not to access physical memory.
    unsafe { IoMemAllocatorBuilder::new(ranges) }
}

/// Port I/O definition reference: <https://bochs.sourceforge.io/techspec/PORTS.LST>.
pub const MAX_IO_PORT: u16 = u16::MAX;
