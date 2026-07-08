// SPDX-License-Identifier: MPL-2.0

//! The RISC-V boot module defines the entrypoints of Asterinas.

pub mod smp;

use core::arch::global_asm;
use core::sync::atomic::{AtomicBool, Ordering};

use fdt::Fdt;
use spin::Once;

use crate::{
    boot::{
        memory_region::{MemoryRegion, MemoryRegionArray, MemoryRegionType},
        BootloaderAcpiArg, BootloaderFramebufferArg,
    },
    early_println,
    mm::paddr_to_vaddr,
};

global_asm!(include_str!("boot.S"));

/// Whether the full kernel page table is active.  During early boot
/// (before init_kernel_page_table), the boot page table may not
/// support LINEAR mapping (QEMU 9.2.4 Sv48 bug), so paddr_to_vaddr()
/// must use identity mapping instead.
pub static KERNEL_PT_READY: AtomicBool = AtomicBool::new(false);

/// The Flattened Device Tree of the platform.
pub static DEVICE_TREE: Once<Fdt> = Once::new();

fn parse_bootloader_name() -> &'static str {
    "Unknown"
}

fn parse_kernel_commandline() -> &'static str {
    DEVICE_TREE.get().unwrap().chosen().bootargs().unwrap_or("")
}

fn parse_initramfs() -> Option<&'static [u8]> {
    let Some((start, end)) = parse_initramfs_range() else {
        return None;
    };

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

    if let Some(node) = DEVICE_TREE.get().unwrap().find_node("/reserved-memory") {
        for child in node.children() {
            if let Some(reg_iter) = child.reg() {
                for region in reg_iter {
                    regions
                        .push(MemoryRegion::new(
                            region.starting_address as usize,
                            region.size.unwrap(),
                            MemoryRegionType::Reserved,
                        ))
                        .unwrap();
                }
            }
        }
    }

    regions.push(MemoryRegion::kernel()).unwrap();

    if let Some((start, end)) = parse_initramfs_range() {
        regions
            .push(MemoryRegion::new(
                start,
                end - start,
                MemoryRegionType::Module,
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

/// Fill the global EARLY_INFO structure.  Called early in crate::init(),
/// before any function that reads EARLY_INFO.  All parse functions here
/// access the already-parsed DTB through identity mapping — they never
/// call paddr_to_vaddr() except in parse_initramfs(), and that path is
/// only entered when the DTB has a chosen/linux,initrd-start property.
#[doc(hidden)]
pub fn fill_early_info() {
    use crate::boot::{EarlyBootInfo, EARLY_INFO};
    EARLY_INFO.call_once(|| EarlyBootInfo {
        bootloader_name: parse_bootloader_name(),
        kernel_cmdline: parse_kernel_commandline(),
        initramfs: parse_initramfs(),
        acpi_arg: parse_acpi_arg(),
        framebuffer_arg: parse_framebuffer_info(),
        memory_regions: parse_memory_regions(),
    });
}

/// The entry point of the Rust code portion of Asterinas.
#[no_mangle]
pub extern "C" fn riscv_boot(_hart_id: usize, device_tree_paddr: usize) -> ! {
    // Direct UART write 'C' — confirms Rust entry with VMA active.
    unsafe { core::ptr::write_volatile(0x10000000 as *mut u8, b'C') };

    // Parse DTB through identity mapping. Timer already disabled in boot.S.
    // device_tree_paddr is in a1 from boot.S; save it before compiler reuses a1.
    let dtb = device_tree_paddr;
    let fdt = unsafe { fdt::Fdt::from_ptr(dtb as *const u8).unwrap() };
    // Direct UART write 'R' — DTB parsed successfully.
    unsafe { core::ptr::write_volatile(0x10000000 as *mut u8, b'R') };
    DEVICE_TREE.call_once(|| fdt);

    use crate::boot::call_ostd_main;
    call_ostd_main();
}
