// SPDX-License-Identifier: MPL-2.0

//! The Linux 64-bit Boot Protocol supporting module.
//!

use core::ffi::CStr;

use linux_boot_params::{BootParams, E820Type, LINUX_BOOT_HEADER_MAGIC};

use crate::{
    boot::{
        memory_region::{MemoryRegion, MemoryRegionArray, MemoryRegionType},
        BootloaderAcpiArg, BootloaderFramebufferArg,
    },
    mm::kspace::{paddr_to_vaddr, LINEAR_MAPPING_BASE_VADDR},
};

fn parse_bootloader_name(boot_params: &BootParams) -> &str {
    let hdr = &boot_params.hdr;
    // The bootloaders have assigned IDs in Linux, see
    // https://www.kernel.org/doc/Documentation/x86/boot.txt
    // for details.
    match hdr.type_of_loader {
        0x0 => "LILO", // (0x00 reserved for pre-2.00 bootloader)
        0x1 => "Loadlin",
        0x2 => "bootsect-loader", // (0x20, all other values reserved)
        0x3 => "Syslinux",
        0x4 => "Etherboot/gPXE/iPXE",
        0x5 => "ELILO",
        0x7 => "GRUB",
        0x8 => "U-Boot",
        0x9 => "Xen",
        0xA => "Gujin",
        0xB => "Qemu",
        0xC => "Arcturus Networks uCbootloader",
        0xD => "kexec-tools",
        0xE => "Extended loader",
        0xF => "Special", // (0xFF = undefined)
        0x10 => "Reserved",
        0x11 => "Minimal Linux Bootloader <http://sebastian-plotz.blogspot.de>",
        0x12 => "OVMF UEFI virtualization stack",
        _ => "Unknown bootloader type!",
    }
}

fn parse_kernel_commandline(boot_params: &BootParams) -> &str {
    // SAFETY: The pointer in the header points to a valid C string.
    let cmdline_c_str: &CStr = unsafe { CStr::from_ptr(boot_params.hdr.cmd_line_ptr as *const i8) };
    let cmdline_str = cmdline_c_str.to_str().unwrap();
    cmdline_str
}

fn parse_initramfs(boot_params: &BootParams) -> Option<&[u8]> {
    let hdr = &boot_params.hdr;
    let ptr = hdr.ramdisk_image as usize;
    if ptr == 0 {
        return None;
    }
    // We must return a slice composed by VA since kernel should read everything in VA.
    let base_va = if ptr < LINEAR_MAPPING_BASE_VADDR {
        paddr_to_vaddr(ptr)
    } else {
        ptr
    };
    let length = hdr.ramdisk_size as usize;
    if length == 0 {
        return None;
    }
    // SAFETY: The regions is reported as initramfs by the bootloader, so it should be valid.
    Some(unsafe { core::slice::from_raw_parts(base_va as *const u8, length) })
}

fn parse_acpi_arg(boot_params: &BootParams) -> BootloaderAcpiArg {
    let rsdp = boot_params.acpi_rsdp_addr;
    if rsdp == 0 {
        BootloaderAcpiArg::NotProvided
    } else {
        BootloaderAcpiArg::Rsdp(rsdp.try_into().expect("RSDP address overflowed!"))
    }
}

fn parse_framebuffer_info(boot_params: &BootParams) -> Option<BootloaderFramebufferArg> {
    let screen_info = boot_params.screen_info;
    if screen_info.lfb_base == 0 {
        return None;
    }
    Some(BootloaderFramebufferArg {
        address: screen_info.lfb_base as usize,
        width: screen_info.lfb_width as usize,
        height: screen_info.lfb_height as usize,
        bpp: screen_info.lfb_depth as usize,
    })
}

impl From<E820Type> for MemoryRegionType {
    fn from(value: E820Type) -> Self {
        match value {
            E820Type::Ram => Self::Usable,
            E820Type::Reserved => Self::Reserved,
            E820Type::Acpi => Self::Reclaimable,
            E820Type::Nvs => Self::NonVolatileSleep,
            _ => Self::BadMemory,
        }
    }
}

fn parse_memory_regions(boot_params: &BootParams) -> MemoryRegionArray {
    let mut regions = MemoryRegionArray::new();

    // Add regions from E820.
    let num_entries = boot_params.e820_entries as usize;
    for e820_entry in &boot_params.e820_table[0..num_entries] {
        regions
            .push(MemoryRegion::new(
                e820_entry.addr as usize,
                e820_entry.size as usize,
                e820_entry.typ.into(),
            ))
            .unwrap();
    }

    // Add the kernel region.
    regions.push(MemoryRegion::kernel()).unwrap();

    // Add the initramfs region.
    regions
        .push(MemoryRegion::new(
            boot_params.hdr.ramdisk_image as usize,
            boot_params.hdr.ramdisk_size as usize,
            MemoryRegionType::Module,
        ))
        .unwrap();

    // Add the AP boot code region that will be copied into by the BSP.
    regions
        .push(MemoryRegion::new(
            super::smp::AP_BOOT_START_PA,
            super::smp::ap_boot_code_size(),
            MemoryRegionType::Reclaimable,
        ))
        .unwrap();

    // Add the kernel cmdline and boot loader name region.
    regions
        .push(MemoryRegion::from_early_str(parse_kernel_commandline(
            boot_params,
        )))
        .unwrap();
    regions
        .push(MemoryRegion::from_early_str(parse_bootloader_name(
            boot_params,
        )))
        .unwrap();

    regions.into_non_overlapping()
}

/// The entry point of the Rust code portion of Asterinas.
#[no_mangle]
unsafe extern "sysv64" fn __linux_boot(params_ptr: *const BootParams) -> ! {
    let params = unsafe { &*params_ptr };
    assert_eq!({ params.hdr.header }, LINUX_BOOT_HEADER_MAGIC);

    use crate::boot::{call_ostd_main, EarlyBootInfo, EARLY_INFO};

    EARLY_INFO.call_once(|| EarlyBootInfo {
        bootloader_name: parse_bootloader_name(params),
        kernel_cmdline: parse_kernel_commandline(params),
        initramfs: parse_initramfs(params),
        acpi_arg: parse_acpi_arg(params),
        framebuffer_arg: parse_framebuffer_info(params),
        memory_regions: parse_memory_regions(params),
    });

    call_ostd_main();
}
