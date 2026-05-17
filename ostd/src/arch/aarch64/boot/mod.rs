// SPDX-License-Identifier: MPL-2.0

//! The ARM64 boot module defines the entrypoints of Asterinas.

pub(crate) mod smp;

use core::arch::global_asm;

use fdt::Fdt;
use spin::Once;

use crate::{
    boot::{
        BootloaderAcpiArg, BootloaderFramebufferArg,
        memory_region::{MemoryRegion, MemoryRegionArray, MemoryRegionType},
    },
    mm::paddr_to_vaddr,
};

global_asm!(include_str!("bsp_boot.S"));

/// The Flattened Device Tree of the platform.
pub static DEVICE_TREE: Once<Fdt> = Once::new();

/// FDT physical address and size, saved for reserving its memory region.
pub static FDT_PHYS: Once<(usize, usize)> = Once::new();

fn parse_bootloader_name() -> &'static str {
    "QEMU virt"
}

fn parse_kernel_commandline() -> &'static str {
    DEVICE_TREE.get().unwrap().chosen().bootargs().unwrap_or("")
}

fn parse_initramfs() -> Option<&'static [u8]> {
    let (start, end) = parse_initramfs_range()?;

    let base_va = paddr_to_vaddr(start);
    let length = end - start;
    Some(unsafe { core::slice::from_raw_parts(base_va as *const u8, length) })
}

fn parse_acpi_arg() -> BootloaderAcpiArg {
    BootloaderAcpiArg::NotProvided
}

fn parse_framebuffer_info() -> Option<BootloaderFramebufferArg> {
    None
}

fn parse_memory_regions() -> MemoryRegionArray {
    let mut regions = MemoryRegionArray::new();

    for region in DEVICE_TREE.get().unwrap().memory().regions() {
        if region.size.unwrap_or(0) > 0 {
            regions
                .push(MemoryRegion::new(
                    region.starting_address as usize,
                    region.size.unwrap(),
                    MemoryRegionType::Usable,
                ))
                .unwrap();
        }
    }

    // Add the kernel region.
    regions.push(MemoryRegion::kernel()).unwrap();

    // Add the initramfs region.
    if let Some((start, end)) = parse_initramfs_range() {
        regions
            .push(MemoryRegion::new(
                start,
                end - start,
                MemoryRegionType::Module,
            ))
            .unwrap();
    }

    // Reserve FDT memory region, like Linux's memblock_reserve(dtb_start, dtb_size).
    // QEMU places the FDT in usable RAM; without reserving it, the frame allocator
    // will reclaim that memory and overwrite the FDT data.
    if let Some((fdt_paddr, fdt_size)) = FDT_PHYS.get() {
        regions
            .push(MemoryRegion::new(
                *fdt_paddr,
                *fdt_size,
                MemoryRegionType::Reserved,
            ))
            .unwrap();
    }

    regions.into_non_overlapping()
}

fn parse_initramfs_range() -> Option<(usize, usize)> {
    let chosen = DEVICE_TREE.get().unwrap().find_node("/chosen").unwrap();
    let initrd_start = chosen.property("linux,initrd-start")?.as_usize()?;
    let initrd_end = chosen.property("linux,initrd-end")?.as_usize()?;
    Some((initrd_start, initrd_end))
}

/// The entry point of the Rust code portion of Asterinas.
///
/// # Safety
///
/// - This function must be called only once at a proper timing in the BSP's boot assembly code.
/// - The caller must follow C calling conventions and put the right arguments in registers.
#[unsafe(no_mangle)]
unsafe extern "C" fn aarch64_boot(fdt_paddr: usize) -> ! {
    let fdt_ptr = paddr_to_vaddr(fdt_paddr) as *const u8;
    let fdt = unsafe { Fdt::from_ptr(fdt_ptr).unwrap() };

    // Save FDT physical address and size for memory reservation.
    // Like Linux's memblock_reserve(dtb_start, dtb_size), we must prevent
    // the frame allocator from reclaiming the FDT region in usable RAM.
    FDT_PHYS.call_once(|| (fdt_paddr, fdt.total_size()));

    DEVICE_TREE.call_once(|| fdt);

    use crate::boot::{EARLY_INFO, EarlyBootInfo, start_kernel};

    EARLY_INFO.call_once(|| EarlyBootInfo {
        bootloader_name: parse_bootloader_name(),
        kernel_cmdline: parse_kernel_commandline(),
        initramfs: parse_initramfs(),
        acpi_arg: parse_acpi_arg(),
        framebuffer_arg: parse_framebuffer_info(),
        memory_regions: parse_memory_regions(),
    });

    unsafe { start_kernel() };
}
