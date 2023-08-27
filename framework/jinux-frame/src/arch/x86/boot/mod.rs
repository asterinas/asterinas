//! The boot module defines the entrypoints of Jinux and the corresponding
//! headers for different bootloaders.
//!
//! We currently support Multiboot2. The support for Linux Boot Protocol is
//! on its way.
//!

mod multiboot;
mod multiboot2;

use core::arch::global_asm;

use alloc::{string::String, vec::Vec};
use spin::Once;

use crate::boot::{
    kcmdline::KCmdlineArg, memory_region::MemoryRegion, BootloaderAcpiArg, BootloaderFramebufferArg,
};

use self::{
    multiboot::{multiboot_entry, MULTIBOOT_ENTRY_MAGIC},
    multiboot2::{multiboot2_entry, MULTIBOOT2_ENTRY_MAGIC},
};

/// Initialize the global boot static varaiables in the boot module to allow
/// other modules to get the boot information.
pub fn init_boot_args(
    bootloader_name: &'static Once<String>,
    kernel_cmdline: &'static Once<KCmdlineArg>,
    initramfs: &'static Once<&'static [u8]>,
    acpi: &'static Once<BootloaderAcpiArg>,
    framebuffer_arg: &'static Once<BootloaderFramebufferArg>,
    memory_regions: &'static Once<Vec<MemoryRegion>>,
) {
    if multiboot::boot_by_multiboot() {
        multiboot::init_boot_args(
            bootloader_name,
            kernel_cmdline,
            initramfs,
            acpi,
            framebuffer_arg,
            memory_regions,
        );
    } else if multiboot2::boot_by_multiboot2() {
        multiboot2::init_boot_args(
            bootloader_name,
            kernel_cmdline,
            initramfs,
            acpi,
            framebuffer_arg,
            memory_regions,
        );
    }
}

global_asm!(include_str!("boot.S"));

#[no_mangle]
unsafe extern "C" fn __boot_entry(boot_magic: u32, boot_params: u64) -> ! {
    match boot_magic {
        MULTIBOOT2_ENTRY_MAGIC => multiboot2_entry(boot_magic, boot_params),
        MULTIBOOT_ENTRY_MAGIC => multiboot_entry(boot_magic, boot_params),
        _ => panic!("Unknown boot magic:{:x?}", boot_magic),
    }
}
