// SPDX-License-Identifier: MPL-2.0

use core::arch::global_asm;

use multiboot2::{BootInformation, BootInformationHeader, MemoryAreaType};
use spin::Once;

use crate::{
    boot::{
        memory_region::{MemoryRegion, MemoryRegionArray, MemoryRegionType},
        BootloaderAcpiArg, BootloaderFramebufferArg,
    },
    mm::kspace::paddr_to_vaddr,
};

global_asm!(include_str!("header.S"));

pub(super) const MULTIBOOT2_ENTRY_MAGIC: u32 = 0x36d76289;

static MB2_INFO: Once<BootInformation> = Once::new();

fn parse_bootloader_name() -> &'static str {
    MB2_INFO
        .get()
        .unwrap()
        .boot_loader_name_tag()
        .expect("Bootloader name not found from the Multiboot2 header!")
        .name()
        .expect("UTF-8 error: failed to parse bootloader name!")
}

fn parse_kernel_commandline() -> &'static str {
    MB2_INFO
        .get()
        .unwrap()
        .command_line_tag()
        .expect("Kernel command-line not found from the Multiboot2 header!")
        .cmdline()
        .expect("UTF-8 error: failed to parse kernel command-line!")
}

fn parse_initramfs() -> Option<&'static [u8]> {
    let mb2_module_tag = MB2_INFO.get().unwrap().module_tags().next()?;
    let base_addr = mb2_module_tag.start_address() as usize;
    // We must return a slice composed by VA since kernel should read everything in VA.
    let base_va = paddr_to_vaddr(base_addr);
    let length = mb2_module_tag.module_size() as usize;
    Some(unsafe { core::slice::from_raw_parts(base_va as *const u8, length) })
}

fn parse_acpi_arg() -> BootloaderAcpiArg {
    if let Some(v2_tag) = MB2_INFO.get().unwrap().rsdp_v2_tag() {
        // check for rsdp v2
        BootloaderAcpiArg::Xsdt(v2_tag.xsdt_address())
    } else if let Some(v1_tag) = MB2_INFO.get().unwrap().rsdp_v1_tag() {
        // fall back to rsdp v1
        BootloaderAcpiArg::Rsdt(v1_tag.rsdt_address())
    } else {
        BootloaderAcpiArg::NotProvided
    }
}

fn parse_framebuffer_info() -> Option<BootloaderFramebufferArg> {
    let Some(Ok(fb_tag)) = MB2_INFO.get().unwrap().framebuffer_tag() else {
        return None;
    };
    Some(BootloaderFramebufferArg {
        address: fb_tag.address() as usize,
        width: fb_tag.width() as usize,
        height: fb_tag.height() as usize,
        bpp: fb_tag.bpp() as usize,
    })
}

impl From<MemoryAreaType> for MemoryRegionType {
    fn from(value: MemoryAreaType) -> Self {
        match value {
            MemoryAreaType::Available => Self::Usable,
            MemoryAreaType::Reserved => Self::Reserved,
            MemoryAreaType::AcpiAvailable => Self::Reclaimable,
            MemoryAreaType::ReservedHibernate => Self::NonVolatileSleep,
            _ => Self::BadMemory,
        }
    }
}

fn parse_memory_regions() -> MemoryRegionArray {
    let mut regions = MemoryRegionArray::new();

    let mb2_info = MB2_INFO.get().unwrap();

    // Add the regions returned by Grub.
    let memory_regions_tag = mb2_info
        .memory_map_tag()
        .expect("Memory region not found from the Multiboot2 header!");
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

    if let Some(Ok(fb_tag)) = mb2_info.framebuffer_tag() {
        // Add the framebuffer region since Grub does not specify it.
        let fb = BootloaderFramebufferArg {
            address: fb_tag.address() as usize,
            width: fb_tag.width() as usize,
            height: fb_tag.height() as usize,
            bpp: fb_tag.bpp() as usize,
        };
        regions
            .push(MemoryRegion::new(
                fb.address,
                (fb.width * fb.height * fb.bpp + 7) / 8, // round up when divide with 8 (bits/Byte)
                MemoryRegionType::Framebuffer,
            ))
            .unwrap();
    }

    // Add the kernel region since Grub does not specify it.
    regions.push(MemoryRegion::kernel()).unwrap();

    // Add the boot module region since Grub does not specify it.
    let mb2_module_tag = mb2_info.module_tags();
    for module in mb2_module_tag {
        regions
            .push(MemoryRegion::new(
                module.start_address() as usize,
                module.module_size() as usize,
                MemoryRegionType::Module,
            ))
            .unwrap();
    }

    // Add the AP boot code region that will be copied into by the BSP.
    regions
        .push(MemoryRegion::new(
            super::smp::AP_BOOT_START_PA,
            super::smp::ap_boot_code_size(),
            MemoryRegionType::Reclaimable,
        ))
        .unwrap();

    // Add the kernel cmdline and boot loader name region since Grub does not specify it.
    regions
        .push(MemoryRegion::from_early_str(parse_kernel_commandline()))
        .unwrap();
    regions
        .push(MemoryRegion::from_early_str(parse_bootloader_name()))
        .unwrap();

    regions.into_non_overlapping()
}

/// The entry point of Rust code called by inline asm.
#[no_mangle]
unsafe extern "sysv64" fn __multiboot2_entry(boot_magic: u32, boot_params: u64) -> ! {
    assert_eq!(boot_magic, MULTIBOOT2_ENTRY_MAGIC);
    MB2_INFO.call_once(|| unsafe {
        BootInformation::load(boot_params as *const BootInformationHeader).unwrap()
    });

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
