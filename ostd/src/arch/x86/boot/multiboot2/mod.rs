// SPDX-License-Identifier: MPL-2.0

use core::arch::global_asm;

use multiboot2::{BootInformation, BootInformationHeader, MemoryAreaType};

use crate::{
    boot::{
        memory_region::{MemoryRegion, MemoryRegionArray, MemoryRegionType},
        BootloaderAcpiArg, BootloaderFramebufferArg,
    },
    mm::{kspace::paddr_to_vaddr, Paddr},
};

global_asm!(include_str!("header.S"));

fn parse_bootloader_name(mb2_info: &BootInformation) -> Option<&'static str> {
    let name = mb2_info.boot_loader_name_tag()?.name().ok()?;

    // SAFETY: The address of `name` is physical and the bootloader name will live for `'static`.
    Some(unsafe { make_str_vaddr_static(name) })
}

fn parse_kernel_commandline(mb2_info: &BootInformation) -> Option<&'static str> {
    let cmdline = mb2_info.command_line_tag()?.cmdline().ok()?;

    // SAFETY: The address of `cmdline` is physical and the command line will live for `'static`.
    Some(unsafe { make_str_vaddr_static(cmdline) })
}

unsafe fn make_str_vaddr_static(str: &str) -> &'static str {
    let vaddr = paddr_to_vaddr(str.as_ptr() as Paddr);

    // SAFETY: The safety is upheld by the caller.
    let bytes = unsafe { core::slice::from_raw_parts(vaddr as *const u8, str.len()) };

    core::str::from_utf8(bytes).unwrap()
}

fn parse_initramfs(mb2_info: &BootInformation) -> Option<&'static [u8]> {
    let module_tag = mb2_info.module_tags().next()?;

    let initramfs_ptr = paddr_to_vaddr(module_tag.start_address() as usize);
    let initramfs_len = module_tag.module_size() as usize;
    // SAFETY: The initramfs is safe to read because of the contract with the loader.
    let initramfs =
        unsafe { core::slice::from_raw_parts(initramfs_ptr as *const u8, initramfs_len) };

    Some(initramfs)
}

fn parse_acpi_arg(mb2_info: &BootInformation) -> BootloaderAcpiArg {
    if let Some(v2_tag) = mb2_info.rsdp_v2_tag() {
        // Check for RSDP v2
        BootloaderAcpiArg::Xsdt(v2_tag.xsdt_address())
    } else if let Some(v1_tag) = mb2_info.rsdp_v1_tag() {
        // Fall back to RSDP v1
        BootloaderAcpiArg::Rsdt(v1_tag.rsdt_address())
    } else {
        BootloaderAcpiArg::NotProvided
    }
}

fn parse_framebuffer_info(mb2_info: &BootInformation) -> Option<BootloaderFramebufferArg> {
    let fb_tag = mb2_info.framebuffer_tag()?.ok()?;

    Some(BootloaderFramebufferArg {
        address: fb_tag.address() as usize,
        width: fb_tag.width() as usize,
        height: fb_tag.height() as usize,
        bpp: fb_tag.bpp() as usize,
        // FIXME: Add the correct color information from the framebuffer tag.
        red_size: 0,
        red_pos: 0,
        green_size: 0,
        green_pos: 0,
        blue_size: 0,
        blue_pos: 0,
        reserved_size: 0,
        reserved_pos: 0,
    })
}

impl From<MemoryAreaType> for MemoryRegionType {
    fn from(value: MemoryAreaType) -> Self {
        match value {
            MemoryAreaType::Available => Self::Usable,
            MemoryAreaType::Reserved => Self::Reserved,
            MemoryAreaType::AcpiAvailable => Self::Reclaimable,
            MemoryAreaType::ReservedHibernate => Self::NonVolatileSleep,
            MemoryAreaType::Defective => Self::BadMemory,
            MemoryAreaType::Custom(_) => Self::Reserved,
        }
    }
}

fn parse_memory_regions(mb2_info: &BootInformation) -> MemoryRegionArray {
    let mut regions = MemoryRegionArray::new();

    // Add the regions returned by Grub.
    let memory_regions_tag = mb2_info
        .memory_map_tag()
        .expect("No memory regions are found in the Multiboot2 header!");
    for region in memory_regions_tag.memory_areas() {
        let start = region.start_address();
        let end = region.end_address();
        let area_typ: MemoryRegionType = MemoryAreaType::from(region.typ()).into();
        let region = MemoryRegion::new(
            start.try_into().unwrap(),
            (end - start).try_into().unwrap(),
            area_typ,
        );
        regions.push(region).unwrap();
    }

    // Add the framebuffer region since Grub does not specify it.
    if let Some(fb) = parse_framebuffer_info(mb2_info) {
        regions.push(MemoryRegion::framebuffer(&fb)).unwrap();
    }

    // Add the kernel region since Grub does not specify it.
    regions.push(MemoryRegion::kernel()).unwrap();

    // Add the initramfs region.
    if let Some(initramfs) = parse_initramfs(mb2_info) {
        regions.push(MemoryRegion::module(initramfs)).unwrap();
    }

    // Add the AP boot code region that will be copied into by the BSP.
    regions
        .push(super::smp::reclaimable_memory_region())
        .unwrap();

    // Add the kernel cmdline and boot loader name region since Grub does not specify it.
    if let Some(kcmdline) = parse_kernel_commandline(mb2_info) {
        regions
            .push(MemoryRegion::module(kcmdline.as_bytes()))
            .unwrap();
    }
    if let Some(bootloader_name) = parse_bootloader_name(mb2_info) {
        regions
            .push(MemoryRegion::module(bootloader_name.as_bytes()))
            .unwrap();
    }

    regions.into_non_overlapping()
}

/// The entry point of Rust code called by inline asm.
#[no_mangle]
unsafe extern "sysv64" fn __multiboot2_entry(boot_magic: u32, boot_params: u64) -> ! {
    assert_eq!(boot_magic, multiboot2::MAGIC);
    let mb2_info =
        unsafe { BootInformation::load(boot_params as *const BootInformationHeader).unwrap() };

    use crate::boot::{call_ostd_main, EarlyBootInfo, EARLY_INFO};

    EARLY_INFO.call_once(|| EarlyBootInfo {
        bootloader_name: parse_bootloader_name(&mb2_info).unwrap_or("Unknown Multiboot2 Loader"),
        kernel_cmdline: parse_kernel_commandline(&mb2_info).unwrap_or(""),
        initramfs: parse_initramfs(&mb2_info),
        acpi_arg: parse_acpi_arg(&mb2_info),
        framebuffer_arg: parse_framebuffer_info(&mb2_info),
        memory_regions: parse_memory_regions(&mb2_info),
    });

    call_ostd_main();
}
