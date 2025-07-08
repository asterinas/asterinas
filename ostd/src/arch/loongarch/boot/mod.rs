// SPDX-License-Identifier: MPL-2.0

//! The LoongArch boot module defines the entrypoints of Asterinas.

mod efi;
pub mod smp;
use core::{arch::global_asm, ffi::CStr};

use fdt::Fdt;
use spin::Once;

use crate::{
    arch::boot::efi::EfiSystemTable,
    boot::{
        memory_region::{MemoryRegion, MemoryRegionArray, MemoryRegionType},
        BootloaderAcpiArg, BootloaderFramebufferArg,
    },
    mm::paddr_to_vaddr,
};

global_asm!(include_str!("boot.S"));

static EFI_SYSTEM_TABLE: Once<&'static EfiSystemTable> = Once::new();

/// The Flattened Device Tree of the platform.
pub static DEVICE_TREE: Once<Fdt> = Once::new();

fn parse_bootloader_name() -> &'static str {
    "Unknown"
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

    regions.into_non_overlapping()
}

fn parse_initramfs_range() -> Option<(usize, usize)> {
    EFI_SYSTEM_TABLE.get().unwrap().initrd()?.range()
}

/// Checks the LoongArch CPU configuration using `cpucfg` instruction.
fn check_cpu_config() {
    let palen = loongArch64::cpu::get_palen();
    let valen = loongArch64::cpu::get_valen();
    let support_iocsr = loongArch64::cpu::get_support_iocsr();

    // Now we only support the 48 bits PA width.
    assert_eq!(palen, 48);
    // Now we only support the 48 bits VA width.
    assert_eq!(valen, 48);
    // Now we require IOCSR support be present.
    assert!(support_iocsr);
}

/// The entry point of the Rust code portion of Asterinas.
///
/// Reference: <https://docs.kernel.org/arch/loongarch/booting.html#information-passed-from-bootloader-to-kernel>
#[no_mangle]
pub extern "C" fn loongarch_boot(_efi_boot: usize, cmdline_paddr: usize, systab_paddr: usize) -> ! {
    check_cpu_config();

    let systab_ptr = paddr_to_vaddr(systab_paddr) as *const EfiSystemTable;
    let systab = unsafe { &*(systab_ptr) };
    EFI_SYSTEM_TABLE.call_once(|| systab);

    let device_tree_ptr =
        paddr_to_vaddr(systab.device_tree().expect("device tree not found")) as *const u8;
    let fdt = unsafe { fdt::Fdt::from_ptr(device_tree_ptr).unwrap() };
    DEVICE_TREE.call_once(|| fdt);

    let cmdline_ptr = paddr_to_vaddr(cmdline_paddr) as *const i8;
    let cmdline = unsafe { CStr::from_ptr(cmdline_ptr) }.to_str();

    use crate::boot::{call_ostd_main, EarlyBootInfo, EARLY_INFO};

    EARLY_INFO.call_once(|| EarlyBootInfo {
        bootloader_name: parse_bootloader_name(),
        kernel_cmdline: cmdline.unwrap_or(""),
        initramfs: parse_initramfs(),
        acpi_arg: parse_acpi_arg(),
        framebuffer_arg: parse_framebuffer_info(),
        memory_regions: parse_memory_regions(),
    });

    call_ostd_main();
}
