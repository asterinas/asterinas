// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;

use align_ext::AlignExt;

use crate::{
    boot::memory_region::MemoryRegionType,
    io::{IoMemAllocatorBuilder, IoPortAllocatorBuilder},
};

/// Initializes the allocatable MMIO area based on the x86-64 memory distribution map.
///
/// In x86-64, the available physical memory area is divided into two regions below 32 bits (Low memory)
/// and above (High memory). The area from the top of low memory to 0xffff_ffff and the area after the
/// top of high memory are available MMIO areas.
pub(super) fn construct_io_mem_allocator_builder() -> IoMemAllocatorBuilder {
    // TODO: Add MMIO regions below 1MB (e.g., VGA framebuffer).
    let regions = &crate::boot::EARLY_INFO.get().unwrap().memory_regions;
    let mut ranges = Vec::with_capacity(2);

    let reserved_filter = regions.iter().filter(|r| {
        r.typ() != (MemoryRegionType::Unknown) && r.typ() != (MemoryRegionType::Reserved)
    });

    // Find the TOLM (Top of Low Memory) and initialize Low MMIO region (TOLM ~ LOW_MMIO_TOP).
    // Align start address to LOW_MMIO_ALIGN
    const LOW_MMIO_TOP: usize = 0x1_0000_0000;
    const LOW_MMIO_ALIGN: usize = 0x1000_0000;
    let (lower_half_base, lower_half_len) = reserved_filter
        .clone()
        .filter(|r| r.base() < u32::MAX as usize)
        .max_by(|a, b| a.base().cmp(&b.base()))
        .map(|reg| (reg.base(), reg.len()))
        .unwrap();

    let mmio_start_addr = (lower_half_base + lower_half_len).align_up(LOW_MMIO_ALIGN);
    assert!(mmio_start_addr < LOW_MMIO_TOP);
    ranges.push(mmio_start_addr..LOW_MMIO_TOP);

    // Find the TOHM (Top of High Memory) and initialize High MMIO region.
    // Here, using HIGH_MMIO_TOP as the top of High MMIO region.
    //
    // TODO: Update the High MMIO region in runtime.
    const HIGH_MMIO_TOP: usize = 0x8000_0000_0000;
    const HIGH_MMIO_ALIGN: usize = 0x1_0000_0000;
    let (upper_half_base, upper_half_len) = reserved_filter
        .filter(|r| r.base() >= u32::MAX as usize)
        .max_by(|a, b| a.base().cmp(&b.base()))
        .map(|reg| (reg.base(), reg.len()))
        .unwrap_or((HIGH_MMIO_ALIGN, 0));

    let mmio_start_addr = (upper_half_base + upper_half_len).align_up(HIGH_MMIO_ALIGN);
    assert!(mmio_start_addr < HIGH_MMIO_TOP);
    ranges.push(mmio_start_addr..HIGH_MMIO_TOP);

    // SAFETY: The range is guaranteed not to access physical memory.
    unsafe { IoMemAllocatorBuilder::new(ranges) }
}

/// Initializes the allocatable PIO area outside OSTD based on the x86-64 port distribution map.
pub(super) fn construct_io_port_allocator_builder() -> IoPortAllocatorBuilder {
    /// Port I/O definition reference: https://bochs.sourceforge.io/techspec/PORTS.LST
    const MAX_IO_PORT: u16 = u16::MAX;

    // SAFETY: `MAX_IO_PORT` is guaranteed not to exceed the maximum value specified by x86-64.
    let mut builder = unsafe { IoPortAllocatorBuilder::new(MAX_IO_PORT) };

    // Device access using Port I/O in OSTD:
    // 1. CMOS      (0x70 ~ 0x72)
    // 2. Serial    (0x3F8 ~ 0x400)
    // 3. PIC       (0x20 ~ 0x22, 0xA0 ~ 0xA2)
    // 4. PIT       (0x40 ~ 0x44)
    //
    // These regions are used in static variables, and will never be dropped.

    builder.remove(0x70..0x72);
    builder.remove(0x3F8..0x400);
    builder.remove(0x20..0x22);
    builder.remove(0xA0..0xA2);
    builder.remove(0x40..0x44);

    builder
}
