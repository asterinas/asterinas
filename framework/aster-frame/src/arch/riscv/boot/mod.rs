// SPDX-License-Identifier: MPL-2.0

use core::arch::global_asm;

use alloc::{string::String, vec::Vec};
use fdt::Fdt;
use spin::Once;

use crate::{boot::{kcmdline::KCmdlineArg, memory_region::{non_overlapping_regions_from, MemoryRegion, MemoryRegionType}, BootloaderAcpiArg, BootloaderFramebufferArg}, early_println, vm::paddr_to_vaddr};

const MAX_HART: usize = 1;
const BOOT_STACK_HART_SIZE: usize = 0x1000 * 32;
const BOOT_STACK_SIZE: usize = BOOT_STACK_HART_SIZE * MAX_HART;

global_asm!(
    include_str!("boot.S"),
    BOOT_STACK_HART_SHIFT = const { BOOT_STACK_HART_SIZE.trailing_zeros() },
    BOOT_STACK_SIZE = const BOOT_STACK_SIZE
);

pub static DEVICE_TREE: Once<Fdt> = Once::new();

fn init_bootloader_name(bootloader_name: &'static Once<String>) {
    bootloader_name.call_once(|| "Unknown".into());
}

fn init_kernel_commandline(kernel_cmdline: &'static Once<KCmdlineArg>) {
    let bootargs = DEVICE_TREE.get().unwrap().chosen().bootargs().unwrap_or("");
    kernel_cmdline.call_once(|| bootargs.into());
}

fn init_initramfs(initramfs: &'static Once<&'static [u8]>) {
    let chosen = DEVICE_TREE.get().unwrap().find_node("/chosen").unwrap();
    let initrd_start = chosen.property("linux,initrd-start").unwrap().as_usize().unwrap();
    let initrd_end = chosen.property("linux,initrd-end").unwrap().as_usize().unwrap();

    let base_va = paddr_to_vaddr(initrd_start);
    let length = initrd_end - initrd_start;
    initramfs.call_once(|| unsafe { core::slice::from_raw_parts(base_va as *const u8, length) });
}

fn init_acpi_arg(acpi: &'static Once<BootloaderAcpiArg>) {
    acpi.call_once(|| BootloaderAcpiArg::NotProvided);
}

fn init_framebuffer_info(framebuffer_arg: &'static Once<BootloaderFramebufferArg>) {
}

fn init_memory_regions(memory_regions: &'static Once<Vec<MemoryRegion>>) {
    let mut regions = Vec::<MemoryRegion>::new();

    for region in DEVICE_TREE.get().unwrap().memory().regions() {
        if region.size.unwrap_or(0) > 0 {
            regions.push(MemoryRegion::new(
                region.starting_address as usize,
                region.size.unwrap(),
                MemoryRegionType::Usable,
            ));
        }
    }

    if let Some(node) = DEVICE_TREE.get().unwrap().find_node("/reserved-memory") {
        for child in node.children() {
            if let Some(reg_iter) = child.reg() {
                for region in reg_iter {
                    regions.push(MemoryRegion::new(
                        region.starting_address as usize,
                        region.size.unwrap(),
                        MemoryRegionType::Reserved,
                    ));
                }
            }
        }
    }

    // Add the kernel region.
    regions.push(MemoryRegion::kernel());

    // Add the initramfs region.
    let chosen = DEVICE_TREE.get().unwrap().find_node("/chosen").unwrap();
    let initrd_start = chosen.property("linux,initrd-start").unwrap().as_usize().unwrap();
    let initrd_end = chosen.property("linux,initrd-end").unwrap().as_usize().unwrap();
    let length = initrd_end - initrd_start;
    regions.push(MemoryRegion::new(
        initrd_start,
        length,
        MemoryRegionType::Module,
    ));

    early_println!("regions: {:#x?}", non_overlapping_regions_from(regions.as_ref()));
    memory_regions.call_once(|| non_overlapping_regions_from(regions.as_ref()));
}

#[no_mangle]
pub extern "C" fn riscv_boot(hart_id: usize, device_tree_paddr: usize) -> ! {
    early_println!("Enter riscv_boot");

    let device_tree_ptr = paddr_to_vaddr(device_tree_paddr) as *const u8;
    let fdt = unsafe { fdt::Fdt::from_ptr(device_tree_ptr).unwrap() };
    early_println!("fdt: {:?}", fdt);
    DEVICE_TREE.call_once(|| fdt);

    crate::boot::register_boot_init_callbacks(
        init_bootloader_name,
        init_kernel_commandline,
        init_initramfs,
        init_acpi_arg,
        init_framebuffer_info,
        init_memory_regions,
    );

    crate::boot::call_aster_main();
}
