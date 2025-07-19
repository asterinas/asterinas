// SPDX-License-Identifier: MPL-2.0

//! The LoongArch boot module defines the entrypoints of Asterinas.

pub mod smp;

use core::arch::global_asm;

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

/// The binary of the Device Tree of the qemu-system-loongarch64 platform.
static DEVICE_TREE_BIN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/loongarch_virt.dtb"));

/// The Flattened Device Tree of the platform.
pub static DEVICE_TREE: Once<Fdt> = Once::new();

fn parse_bootloader_name() -> &'static str {
    "Unknown"
}

fn parse_kernel_commandline() -> &'static str {
    "SHELL=/bin/sh LOGNAME=root HOME=/ USER=root PATH=/bin init=/bin/busybox ostd.log_level=trace -- sh -l"
}

fn parse_initramfs() -> Option<&'static [u8]> {
    None
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

    regions.into_non_overlapping()
}

/// Print the LoongArch CPU configuration using `cpucfg` instruction.
fn print_cpu_config() {
    let prid = loongArch64::cpu::get_prid();
    let palen = loongArch64::cpu::get_palen();
    // Now we only support the 48 bits PA width.
    assert!(palen == 48);
    let valen = loongArch64::cpu::get_valen();
    // Now we only support the 48 bits VA width.
    assert!(valen == 48);
    let mmu_support_page = loongArch64::cpu::get_mmu_support_page();
    let support_huge_page = loongArch64::cpu::get_support_huge_page();
    let save_num = loongArch64::register::prcfg1::read().save_num();
    let support_iocsr = loongArch64::cpu::get_support_iocsr();
    assert!(support_iocsr);

    early_println!("");
    early_println!("LoongArch CPU Configuration:");
    early_println!("  PRID: 0x{:x}", prid);
    early_println!("  PA Width: {} bits", palen);
    early_println!("  VA Width: {} bits", valen);
    early_println!("  MMU Support Page: {}", mmu_support_page);
    early_println!("  Support Huge Page: {}", support_huge_page);
    early_println!("  CSR Save Num: {}", save_num);
    early_println!("  Support IOCSR: {}", support_iocsr);
    early_println!("");
}

/// Clear the BSS section.
fn clear_bss() {
    unsafe extern "C" {
        unsafe fn __bss();
        unsafe fn __bss_end();
    }

    let bss_start = __bss as usize;
    let bss_end = __bss_end as usize;
    let bss_size = bss_end - bss_start;

    unsafe {
        core::ptr::write_bytes(bss_start as *mut u8, 0, bss_size);
    }
}

/// The entry point of the Rust code portion of Asterinas.
#[no_mangle]
pub extern "C" fn loongarch_boot(_core_id: usize) -> ! {
    early_println!("Enter loongarch_boot");

    clear_bss();

    print_cpu_config();

    DEVICE_TREE.call_once(|| fdt::Fdt::new(DEVICE_TREE_BIN).unwrap());

    use crate::boot::{call_ostd_main, EarlyBootInfo, EARLY_INFO};

    EARLY_INFO.call_once(|| EarlyBootInfo {
        bootloader_name: parse_bootloader_name(),
        kernel_cmdline: parse_kernel_commandline(),
        initramfs: parse_initramfs(),
        acpi_arg: parse_acpi_arg(),
        framebuffer_arg: parse_framebuffer_info(),
        memory_regions: parse_memory_regions(),
    });

    call_ostd_main();
}
