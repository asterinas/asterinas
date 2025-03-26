// SPDX-License-Identifier: MPL-2.0

//! The Linux 64-bit Boot Protocol supporting module.
//!

use linux_boot_params::{BootParams, E820Type, LINUX_BOOT_HEADER_MAGIC};

use crate::{
    arch::init_cvm_guest,
    boot::{
        memory_region::{MemoryRegion, MemoryRegionArray, MemoryRegionType},
        BootloaderAcpiArg, BootloaderFramebufferArg,
    },
    if_tdx_enabled,
    mm::kspace::paddr_to_vaddr,
};

fn parse_bootloader_name(boot_params: &BootParams) -> &str {
    // The bootloaders have assigned IDs in Linux, see
    // https://www.kernel.org/doc/Documentation/x86/boot.txt
    // for details.
    match boot_params.hdr.type_of_loader {
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
        _ => "Unknown Linux Loader",
    }
}

fn parse_kernel_commandline(boot_params: &BootParams) -> Option<&str> {
    if boot_params.ext_cmd_line_ptr != 0 {
        // TODO: We can support the above 4GiB command line after setting up
        // linear mappings. By far, we cannot log the error because the serial is
        // not up. Proceed as if there was no command line.
        return None;
    }

    if boot_params.hdr.cmd_line_ptr == 0 || boot_params.hdr.cmdline_size == 0 {
        return None;
    }

    let cmdline_ptr = paddr_to_vaddr(boot_params.hdr.cmd_line_ptr as usize);
    let cmdline_len = boot_params.hdr.cmdline_size as usize;
    // SAFETY: The command line is safe to read because of the contract with the loader.
    let cmdline = unsafe { core::slice::from_raw_parts(cmdline_ptr as *const u8, cmdline_len) };

    // Now, unfortunately, there are silent errors because the serial is not up.
    core::ffi::CStr::from_bytes_until_nul(cmdline)
        .ok()?
        .to_str()
        .ok()
}

fn parse_initramfs(boot_params: &BootParams) -> Option<&[u8]> {
    if boot_params.ext_ramdisk_image != 0 || boot_params.ext_ramdisk_size != 0 {
        // See the explanation in `parse_kernel_commandline`.
        return None;
    }

    if boot_params.hdr.ramdisk_image == 0 || boot_params.hdr.ramdisk_size == 0 {
        return None;
    }

    let initramfs_ptr = paddr_to_vaddr(boot_params.hdr.ramdisk_image as usize);
    let initramfs_len = boot_params.hdr.ramdisk_size as usize;
    // SAFETY: The initramfs is safe to read because of the contract with the loader.
    let initramfs =
        unsafe { core::slice::from_raw_parts(initramfs_ptr as *const u8, initramfs_len) };

    Some(initramfs)
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

    let address = screen_info.lfb_base as usize | ((screen_info.ext_lfb_base as usize) << 32);
    if address == 0 {
        return None;
    }

    Some(BootloaderFramebufferArg {
        address,
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
            E820Type::Unusable => Self::BadMemory,
            // All other memory regions are reserved.
            // FIXME: Using Rust enum in this way can be unsound if the bootloader passes an
            // unknown memory type to the kernel (e.g., due to a newer protocol version).
            _ => Self::Reserved,
        }
    }
}

fn parse_memory_regions(boot_params: &BootParams) -> MemoryRegionArray {
    let mut regions = MemoryRegionArray::new();

    // Add regions from E820.
    let num_entries = boot_params.e820_entries as usize;
    for e820_entry in &boot_params.e820_table[0..num_entries] {
        if_tdx_enabled!({
            if (e820_entry.addr..(e820_entry.addr + e820_entry.size)).contains(&0x800000) {
                regions
                    .push(MemoryRegion::new(
                        e820_entry.addr as usize,
                        e820_entry.size as usize,
                        MemoryRegionType::Reclaimable,
                    ))
                    .unwrap();
                continue;
            }
        });
        regions
            .push(MemoryRegion::new(
                e820_entry.addr as usize,
                e820_entry.size as usize,
                e820_entry.typ.into(),
            ))
            .unwrap();
    }

    // Add the framebuffer region.
    if let Some(fb) = parse_framebuffer_info(boot_params) {
        regions.push(MemoryRegion::framebuffer(&fb)).unwrap();
    }

    // Add the kernel region.
    regions.push(MemoryRegion::kernel()).unwrap();

    // Add the initramfs region.
    if let Some(initramfs) = parse_initramfs(boot_params) {
        regions.push(MemoryRegion::module(initramfs)).unwrap();
    }

    // Add the AP boot code region that will be copied into by the BSP.
    regions
        .push(super::smp::reclaimable_memory_region())
        .unwrap();

    // Add the region of the kernel cmdline since some bootloaders do not provide it.
    if let Some(kcmdline) = parse_kernel_commandline(boot_params) {
        regions
            .push(MemoryRegion::module(kcmdline.as_bytes()))
            .unwrap();
    }

    regions.into_non_overlapping()
}

/// The entry point of the Rust code portion of Asterinas.
#[no_mangle]
unsafe extern "sysv64" fn __linux_boot(params_ptr: *const BootParams) -> ! {
    let params = unsafe { &*params_ptr };
    assert_eq!({ params.hdr.header }, LINUX_BOOT_HEADER_MAGIC);

    use crate::boot::{call_ostd_main, EarlyBootInfo, EARLY_INFO};

    #[cfg(feature = "cvm_guest")]
    init_cvm_guest();

    EARLY_INFO.call_once(|| EarlyBootInfo {
        bootloader_name: parse_bootloader_name(params),
        kernel_cmdline: parse_kernel_commandline(params).unwrap_or(""),
        initramfs: parse_initramfs(params),
        acpi_arg: parse_acpi_arg(params),
        framebuffer_arg: parse_framebuffer_info(params),
        memory_regions: parse_memory_regions(params),
    });

    call_ostd_main();
}
