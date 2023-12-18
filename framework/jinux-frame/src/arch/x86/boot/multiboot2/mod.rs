use alloc::{
    string::{String, ToString},
    vec::Vec,
};
use multiboot2::{BootInformation, BootInformationHeader, MemoryAreaType};

use crate::boot::{
    kcmdline::KCmdlineArg,
    memory_region::{non_overlapping_regions_from, MemoryRegion, MemoryRegionType},
    BootloaderAcpiArg, BootloaderFramebufferArg,
};
use spin::Once;

use core::arch::global_asm;

global_asm!(include_str!("header.S"));

use crate::{config::PHYS_OFFSET, vm::paddr_to_vaddr};

pub(super) const MULTIBOOT2_ENTRY_MAGIC: u32 = 0x36d76289;

static MB2_INFO: Once<BootInformation> = Once::new();

fn init_bootloader_name(bootloader_name: &'static Once<String>) {
    bootloader_name.call_once(|| {
        MB2_INFO
            .get()
            .unwrap()
            .boot_loader_name_tag()
            .expect("Bootloader name not found from the Multiboot2 header!")
            .name()
            .expect("UTF-8 error: failed to parse bootloader name!")
            .to_string()
    });
}

fn init_kernel_commandline(kernel_cmdline: &'static Once<KCmdlineArg>) {
    kernel_cmdline.call_once(|| {
        MB2_INFO
            .get()
            .unwrap()
            .command_line_tag()
            .expect("Kernel command-line not found from the Multiboot2 header!")
            .cmdline()
            .expect("UTF-8 error: failed to parse kernel command-line!")
            .into()
    });
}

fn init_initramfs(initramfs: &'static Once<&'static [u8]>) {
    let mb2_module_tag = MB2_INFO
        .get()
        .unwrap()
        .module_tags()
        .next()
        .expect("No Multiboot2 modules found!");
    let base_addr = mb2_module_tag.start_address() as usize;
    // We must return a slice composed by VA since kernel should read every in VA.
    let base_va = if base_addr < PHYS_OFFSET {
        paddr_to_vaddr(base_addr)
    } else {
        base_addr
    };
    let length = mb2_module_tag.module_size() as usize;
    initramfs.call_once(|| unsafe { core::slice::from_raw_parts(base_va as *const u8, length) });
}

fn init_acpi_arg(acpi: &'static Once<BootloaderAcpiArg>) {
    acpi.call_once(|| {
        if let Some(v2_tag) = MB2_INFO.get().unwrap().rsdp_v2_tag() {
            // check for rsdp v2
            BootloaderAcpiArg::Xsdt(v2_tag.xsdt_address())
        } else if let Some(v1_tag) = MB2_INFO.get().unwrap().rsdp_v1_tag() {
            // fall back to rsdp v1
            BootloaderAcpiArg::Rsdt(v1_tag.rsdt_address())
        } else {
            panic!("No ACPI RDSP information found!");
        }
    });
}

fn init_framebuffer_info(framebuffer_arg: &'static Once<BootloaderFramebufferArg>) {
    let Some(Ok(fb_tag)) = MB2_INFO.get().unwrap().framebuffer_tag() else {
        return;
    };
    framebuffer_arg.call_once(|| BootloaderFramebufferArg {
        address: fb_tag.address() as usize,
        width: fb_tag.width() as usize,
        height: fb_tag.height() as usize,
        bpp: fb_tag.bpp() as usize,
    });
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

fn init_memory_regions(memory_regions: &'static Once<Vec<MemoryRegion>>) {
    let mut regions = Vec::<MemoryRegion>::new();

    // Add the regions returned by Grub.
    let memory_regions_tag = MB2_INFO
        .get()
        .unwrap()
        .memory_map_tag()
        .expect("Memory region not found from the Multiboot2 header!");
    let num_memory_regions = memory_regions_tag.memory_areas().len();
    for i in 0..num_memory_regions {
        let start = memory_regions_tag.memory_areas()[i].start_address();
        let end = memory_regions_tag.memory_areas()[i].end_address();
        let area_typ: MemoryRegionType = memory_regions_tag.memory_areas()[i].typ().into();
        let region = MemoryRegion::new(
            start.try_into().unwrap(),
            (end - start).try_into().unwrap(),
            area_typ,
        );
        regions.push(region);
    }
    if let Some(Ok(fb_tag)) = MB2_INFO.get().unwrap().framebuffer_tag() {
        // Add the framebuffer region since Grub does not specify it.
        let fb = BootloaderFramebufferArg {
            address: fb_tag.address() as usize,
            width: fb_tag.width() as usize,
            height: fb_tag.height() as usize,
            bpp: fb_tag.bpp() as usize,
        };
        regions.push(MemoryRegion::new(
            fb.address,
            (fb.width * fb.height * fb.bpp + 7) / 8, // round up when divide with 8 (bits/Byte)
            MemoryRegionType::Framebuffer,
        ));
    }
    // Add the kernel region since Grub does not specify it.
    // These are physical addresses provided by the linker script.
    extern "C" {
        fn __kernel_start();
        fn __kernel_end();
    }
    regions.push(MemoryRegion::new(
        __kernel_start as usize,
        __kernel_end as usize - __kernel_start as usize,
        MemoryRegionType::Kernel,
    ));
    // Add the boot module region since Grub does not specify it.
    let mb2_module_tag = MB2_INFO.get().unwrap().module_tags();
    for m in mb2_module_tag {
        regions.push(MemoryRegion::new(
            m.start_address() as usize,
            m.module_size() as usize,
            MemoryRegionType::Module,
        ));
    }

    // Initialize with non-overlapping regions.
    memory_regions.call_once(move || non_overlapping_regions_from(regions.as_ref()));
}

/// The entry point of Rust code called by inline asm.
#[no_mangle]
unsafe extern "sysv64" fn __multiboot2_entry(boot_magic: u32, boot_params: u64) -> ! {
    assert_eq!(boot_magic, MULTIBOOT2_ENTRY_MAGIC);
    MB2_INFO.call_once(|| unsafe {
        BootInformation::load(boot_params as *const BootInformationHeader).unwrap()
    });
    crate::boot::register_boot_init_callbacks(
        init_bootloader_name,
        init_kernel_commandline,
        init_initramfs,
        init_acpi_arg,
        init_framebuffer_info,
        init_memory_regions,
    );
    crate::boot::call_jinux_main();
}
