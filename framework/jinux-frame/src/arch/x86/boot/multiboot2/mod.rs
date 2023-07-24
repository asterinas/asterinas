use alloc::{string::ToString, vec::Vec};
use multiboot2::{BootInformation, BootInformationHeader, MemoryAreaType};

use super::{
    memory_region::{MemoryRegion, MemoryRegionType},
    BootloaderAcpiArg, BootloaderFramebufferArg, ACPI_RSDP, BOOTLOADER_NAME, FRAMEBUFFER_INFO,
    INITRAMFS, KERNEL_COMMANDLINE, MEMORY_REGIONS,
};
use core::{arch::global_asm, mem::swap};
use spin::Once;

use crate::{config::PHYS_OFFSET, vm::paddr_to_vaddr};

global_asm!(include_str!("boot.S"));

const MULTIBOOT2_ENTRY_MAGIC: u32 = 0x36d76289;

static MB2_INFO: Once<BootInformation> = Once::new();

pub fn init_bootloader_name() {
    BOOTLOADER_NAME.call_once(|| {
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

pub fn init_kernel_commandline() {
    KERNEL_COMMANDLINE.call_once(|| {
        MB2_INFO
            .get()
            .unwrap()
            .command_line_tag()
            .expect("Kernel commandline not found from the Multiboot2 header!")
            .cmdline()
            .expect("UTF-8 error: failed to parse kernel commandline!")
            .to_string()
    });
}

pub fn init_initramfs() {
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
    INITRAMFS.call_once(|| unsafe { core::slice::from_raw_parts(base_va as *const u8, length) });
}

pub fn init_acpi_rsdp() {
    ACPI_RSDP.call_once(|| {
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

pub fn init_framebuffer_info() {
    let fb_tag = MB2_INFO.get().unwrap().framebuffer_tag().unwrap().unwrap();
    FRAMEBUFFER_INFO.call_once(|| BootloaderFramebufferArg {
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

pub fn init_memory_regions() {
    // We should later use regions in `regions_unusable` to truncate all
    // regions in `regions_usable`.
    // The difference is that regions in `regions_usable` could be used by
    // the frame allocator.
    let mut regions_usable = Vec::<MemoryRegion>::new();
    let mut regions_unusable = Vec::<MemoryRegion>::new();

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
        match area_typ {
            MemoryRegionType::Usable | MemoryRegionType::Reclaimable => {
                regions_usable.push(region);
            }
            _ => {
                regions_unusable.push(region);
            }
        }
    }
    // Add the framebuffer region since Grub does not specify it.
    let fb_tag = MB2_INFO.get().unwrap().framebuffer_tag().unwrap().unwrap();
    let fb = BootloaderFramebufferArg {
        address: fb_tag.address() as usize,
        width: fb_tag.width() as usize,
        height: fb_tag.height() as usize,
        bpp: fb_tag.bpp() as usize,
    };
    regions_unusable.push(MemoryRegion::new(
        fb.address,
        (fb.width * fb.height * fb.bpp + 7) / 8, // round up when divide with 8 (bits/Byte)
        MemoryRegionType::Framebuffer,
    ));
    // Add the kernel region since Grub does not specify it.
    // These are physical addresses provided by the linker script.
    extern "C" {
        fn __kernel_start();
        fn __kernel_end();
    }
    regions_unusable.push(MemoryRegion::new(
        __kernel_start as usize,
        __kernel_end as usize - __kernel_start as usize,
        MemoryRegionType::Kernel,
    ));
    // Add the boot module region since Grub does not specify it.
    let mb2_module_tag = MB2_INFO.get().unwrap().module_tags();
    for m in mb2_module_tag {
        regions_unusable.push(MemoryRegion::new(
            m.start_address() as usize,
            m.module_size() as usize,
            MemoryRegionType::Module,
        ));
    }

    // `regions_*` are 2 rolling vectors since we are going to truncate
    // the regions in a iterative manner.
    let mut regions = Vec::<MemoryRegion>::new();
    let regions_src = &mut regions_usable;
    let regions_dst = &mut regions;
    // Truncate the usable regions.
    for &r_unusable in &regions_unusable {
        regions_dst.clear();
        for r_usable in &*regions_src {
            regions_dst.append(&mut r_usable.truncate(&r_unusable));
        }
        swap(regions_src, regions_dst);
    }

    // Initialize with regions_unusable + regions_src
    MEMORY_REGIONS.call_once(move || {
        let mut all_regions = regions_unusable;
        all_regions.append(regions_src);
        all_regions
    });
}

// The entry point of kernel code, which should be defined by the package that
// uses jinux-frame.
extern "Rust" {
    fn jinux_main() -> !;
}

/// The entry point of Rust code called by inline asm.
#[no_mangle]
unsafe extern "C" fn __multiboot2_entry(boot_magic: u32, boot_params: u64) -> ! {
    assert_eq!(boot_magic, MULTIBOOT2_ENTRY_MAGIC);
    MB2_INFO.call_once(|| unsafe {
        BootInformation::load(boot_params as *const BootInformationHeader).unwrap()
    });
    jinux_main();
}
