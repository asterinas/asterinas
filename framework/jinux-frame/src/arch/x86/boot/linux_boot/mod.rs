//! The Linux 64-bit Boot Protocol supporting module.
//!

mod boot_params;
use boot_params::E820Type;

use crate::boot::{
    kcmdline::KCmdlineArg,
    memory_region::{MemoryRegion, MemoryRegionType},
    BootloaderAcpiArg, BootloaderFramebufferArg,
};
use crate::{config::PHYS_OFFSET, vm::paddr_to_vaddr};

use alloc::{borrow::ToOwned, format, string::String, vec::Vec};
use core::ffi::CStr;

use spin::Once;

static BOOT_PARAMS: Once<boot_params::BootParams> = Once::new();

fn init_bootloader_name(bootloader_name: &'static Once<String>) {
    let hdr = &BOOT_PARAMS.get().unwrap().hdr;
    // The bootloaders have assigned IDs in Linux, see
    // https://www.kernel.org/doc/Documentation/x86/boot.txt
    // for details.
    let ext_str: String;
    let name = match hdr.type_of_loader {
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
        0xE => {
            // Extended
            ext_str = format!(
                "Extended bootloader {}, version {}",
                (hdr.ext_loader_type + 0x10),
                (hdr.type_of_loader & 0x0f) + (hdr.ext_loader_ver << 4)
            );
            &ext_str
        }
        0xF => "Special", // (0xFF = undefined)
        0x10 => "Reserved",
        0x11 => "Minimal Linux Bootloader <http://sebastian-plotz.blogspot.de>",
        0x12 => "OVMF UEFI virtualization stack",
        _ => "Unknown bootloader type!",
    }
    .to_owned();
    bootloader_name.call_once(|| name);
}

fn init_kernel_commandline(kernel_cmdline: &'static Once<KCmdlineArg>) {
    let cmdline_c_str: &CStr =
        unsafe { CStr::from_ptr(BOOT_PARAMS.get().unwrap().hdr.cmd_line_ptr as *const i8) };
    let cmdline_str = cmdline_c_str.to_str().unwrap();
    kernel_cmdline.call_once(|| cmdline_str.into());
}

fn init_initramfs(initramfs: &'static Once<&'static [u8]>) {
    let hdr = &BOOT_PARAMS.get().unwrap().hdr;
    let ptr = hdr.ramdisk_image as usize;
    // We must return a slice composed by VA since kernel should read everything in VA.
    let base_va = if ptr < PHYS_OFFSET {
        paddr_to_vaddr(ptr)
    } else {
        ptr
    };
    let length = hdr.ramdisk_size as usize;
    initramfs.call_once(|| unsafe { core::slice::from_raw_parts(base_va as *const u8, length) });
}

fn init_acpi_arg(acpi: &'static Once<BootloaderAcpiArg>) {
    let rsdp = BOOT_PARAMS.get().unwrap().acpi_rsdp_addr;
    if rsdp == 0 {
        acpi.call_once(|| BootloaderAcpiArg::NotProvided);
    } else {
        acpi.call_once(|| {
            BootloaderAcpiArg::Rsdp(rsdp.try_into().expect("RSDP address overflowed!"))
        });
    }
}

fn init_framebuffer_info(framebuffer_arg: &'static Once<BootloaderFramebufferArg>) {
    let screen_info = &BOOT_PARAMS.get().unwrap().screen_info;
    framebuffer_arg.call_once(|| BootloaderFramebufferArg {
        address: screen_info.lfb_base as usize,
        width: screen_info.lfb_width as usize,
        height: screen_info.lfb_height as usize,
        bpp: screen_info.lfb_depth as usize,
    });
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

fn init_memory_regions(memory_regions: &'static Once<Vec<MemoryRegion>>) {
    let boot_params = &BOOT_PARAMS.get().unwrap();
    let num_entries = boot_params.e820_entries as usize;
    let mut regions = Vec::<MemoryRegion>::new();
    for e820_entry in &boot_params.e820_table[0..num_entries] {
        regions.push(MemoryRegion::new(
            e820_entry.addr as usize,
            e820_entry.size as usize,
            e820_entry.typ.into(),
        ));
    }
    memory_regions.call_once(|| regions);
}

/// The entry point of Rust code called by the Linux 64-bit boot compatible bootloader.
#[no_mangle]
unsafe extern "sysv64" fn __linux64_boot(params_ptr: *const boot_params::BootParams) -> ! {
    let params = *params_ptr;
    assert_eq!({ params.hdr.header }, boot_params::LINUX_BOOT_HEADER_MAGIC);
    BOOT_PARAMS.call_once(|| params);
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
