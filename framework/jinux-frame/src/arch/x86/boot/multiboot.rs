use core::mem::swap;

use alloc::{string::String, vec::Vec};
use multiboot2::MemoryAreaType;
use spin::Once;

use crate::{
    arch::x86::kernel::acpi::AcpiMemoryHandler,
    boot::{
        kcmdline::KCmdlineArg,
        memory_region::{MemoryRegion, MemoryRegionType},
        BootloaderAcpiArg, BootloaderFramebufferArg,
    },
    config::PHYS_OFFSET,
    vm::paddr_to_vaddr,
};

pub(super) const MULTIBOOT_ENTRY_MAGIC: u32 = 0x2BADB002;

/// Initialize the global boot static varaiables in the boot module to allow
/// other modules to get the boot information.
pub(super) fn init_boot_args(
    bootloader_name: &'static Once<String>,
    kernel_cmdline: &'static Once<KCmdlineArg>,
    initramfs: &'static Once<&'static [u8]>,
    acpi: &'static Once<BootloaderAcpiArg>,
    framebuffer_arg: &'static Once<BootloaderFramebufferArg>,
    memory_regions: &'static Once<Vec<MemoryRegion>>,
) {
    init_bootloader_name(bootloader_name);
    init_kernel_commandline(kernel_cmdline);
    init_initramfs(initramfs);
    init_acpi_arg(acpi);
    init_framebuffer_info(framebuffer_arg);
    init_memory_regions(memory_regions);
}

pub fn boot_by_multiboot() -> bool {
    MB1_INFO.is_completed()
}

fn init_bootloader_name(bootloader_name: &'static Once<String>) {
    bootloader_name.call_once(|| {
        let mut name = "";
        let info = MB1_INFO.get().unwrap();
        if info.boot_loader_name != 0 {
            // Safety: the bootloader name is C-style zero-terminated string.
            unsafe {
                let cstr = paddr_to_vaddr(info.boot_loader_name as usize) as *const u8;
                let mut len = 0;
                while cstr.add(len).read() != 0 {
                    len += 1;
                }

                name = core::str::from_utf8(core::slice::from_raw_parts(cstr, len))
                    .expect("cmdline is not a utf-8 string");
            }
        }
        name.into()
    });
}

fn init_kernel_commandline(kernel_cmdline: &'static Once<KCmdlineArg>) {
    kernel_cmdline.call_once(|| {
        let mut cmdline = "";
        let info = MB1_INFO.get().unwrap();
        if info.cmdline != 0 {
            // Safety: the command line is C-style zero-terminated string.
            unsafe {
                let cstr = paddr_to_vaddr(info.cmdline as usize) as *const u8;
                let mut len = 0;
                while cstr.add(len).read() != 0 {
                    len += 1;
                }

                cmdline = core::str::from_utf8(core::slice::from_raw_parts(cstr, len))
                    .expect("cmdline is not a utf-8 string");
            }
        }
        cmdline.into()
    });
}

fn init_initramfs(initramfs: &'static Once<&'static [u8]>) {
    let info = MB1_INFO.get().unwrap();
    // FIXME: We think all modules are initramfs, can this cause problems?
    if info.mods_count == 0 {
        return;
    }
    let modules_addr = info.mods_addr as usize;
    // We only use one module
    let (start, end) = unsafe {
        (
            (*(paddr_to_vaddr(modules_addr) as *const u32)) as usize,
            (*(paddr_to_vaddr(modules_addr + 4) as *const u32)) as usize,
        )
    };
    // We must return a slice composed by VA since kernel should read every in VA.
    let base_va = if start < PHYS_OFFSET {
        paddr_to_vaddr(start)
    } else {
        start
    };
    let length = end - start;
    initramfs.call_once(|| unsafe { core::slice::from_raw_parts(base_va as *const u8, length) });
}

fn init_acpi_arg(acpi: &'static Once<BootloaderAcpiArg>) {
    // The multiboot protocol does not contain RSDP address.
    // TODO: What about UEFI?
    let rsdp = unsafe { rsdp::Rsdp::search_for_on_bios(AcpiMemoryHandler {}) };
    match rsdp {
        Ok(map) => match map.validate() {
            Ok(_) => acpi.call_once(|| {
                if map.revision() > 0 {
                    BootloaderAcpiArg::Xsdt(map.xsdt_address() as usize)
                } else {
                    BootloaderAcpiArg::Rsdt(map.rsdt_address() as usize)
                }
            }),
            Err(_) => acpi.call_once(|| BootloaderAcpiArg::NotExists),
        },
        Err(_) => acpi.call_once(|| BootloaderAcpiArg::NotExists),
    };
}

fn init_framebuffer_info(framebuffer_arg: &'static Once<BootloaderFramebufferArg>) {
    let info = MB1_INFO.get().unwrap();
    framebuffer_arg.call_once(|| BootloaderFramebufferArg {
        address: info.framebuffer_table.addr as usize,
        width: info.framebuffer_table.width as usize,
        height: info.framebuffer_table.height as usize,
        bpp: info.framebuffer_table.bpp as usize,
    });
}

fn init_memory_regions(memory_regions: &'static Once<Vec<MemoryRegion>>) {
    // We should later use regions in `regions_unusable` to truncate all
    // regions in `regions_usable`.
    // The difference is that regions in `regions_usable` could be used by
    // the frame allocator.
    let mut regions_usable = Vec::<MemoryRegion>::new();
    let mut regions_unusable = Vec::<MemoryRegion>::new();

    // Add the regions in the multiboot protocol.
    let info = MB1_INFO.get().unwrap();
    let start = info.memory_map_addr as usize;
    let length = info.memory_map_len as usize;
    let mut current = start;

    while current < start + length {
        let entry = unsafe { &*(paddr_to_vaddr(current) as *const MemoryEntry) };
        let start = entry.base_addr;
        let area_type: MemoryRegionType = entry.memory_type.into();
        let region = MemoryRegion::new(
            start.try_into().unwrap(),
            entry.length.try_into().unwrap(),
            area_type,
        );
        match area_type {
            MemoryRegionType::Usable | MemoryRegionType::Reclaimable => {
                regions_usable.push(region);
            }
            _ => {
                regions_unusable.push(region);
            }
        }
        current += entry.size as usize + 4;
    }

    // Add the framebuffer region.
    let fb = BootloaderFramebufferArg {
        address: info.framebuffer_table.addr as usize,
        width: info.framebuffer_table.width as usize,
        height: info.framebuffer_table.height as usize,
        bpp: info.framebuffer_table.bpp as usize,
    };
    regions_unusable.push(MemoryRegion::new(
        fb.address,
        (fb.width * fb.height * fb.bpp + 7) / 8, // round up when divide with 8 (bits/Byte)
        MemoryRegionType::Framebuffer,
    ));
    // Add the kernel region.
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

    // Add the initramfs area.
    // These are physical addresses provided by the linker script.
    if info.mods_count != 0 {
        let modules_addr = info.mods_addr as usize;
        // We only use one module
        let (start, end) = unsafe {
            (
                (*(paddr_to_vaddr(modules_addr) as *const u32)) as usize,
                (*(paddr_to_vaddr(modules_addr + 4) as *const u32)) as usize,
            )
        };
        regions_unusable.push(MemoryRegion::new(
            start,
            end - start,
            MemoryRegionType::Reserved,
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
    memory_regions.call_once(move || {
        let mut all_regions = regions_unusable;
        all_regions.append(&mut regions_usable);
        all_regions
    });
}

/// Representation of Multiboot Information according to specification.
///
/// Ref:https://www.gnu.org/software/grub/manual/multiboot/multiboot.html#Boot-information-format
///
///```text
///         +-------------------+
/// 0       | flags             |    (required)
///         +-------------------+
/// 4       | mem_lower         |    (present if flags[0] is set)
/// 8       | mem_upper         |    (present if flags[0] is set)
///         +-------------------+
/// 12      | boot_device       |    (present if flags[1] is set)
///         +-------------------+
/// 16      | cmdline           |    (present if flags[2] is set)
///         +-------------------+
/// 20      | mods_count        |    (present if flags[3] is set)
/// 24      | mods_addr         |    (present if flags[3] is set)
///         +-------------------+
/// 28 - 40 | syms              |    (present if flags[4] or
///         |                   |                flags[5] is set)
///         +-------------------+
/// 44      | mmap_length       |    (present if flags[6] is set)
/// 48      | mmap_addr         |    (present if flags[6] is set)
///         +-------------------+
/// 52      | drives_length     |    (present if flags[7] is set)
/// 56      | drives_addr       |    (present if flags[7] is set)
///         +-------------------+
/// 60      | config_table      |    (present if flags[8] is set)
///         +-------------------+
/// 64      | boot_loader_name  |    (present if flags[9] is set)
///         +-------------------+
/// 68      | apm_table         |    (present if flags[10] is set)
///         +-------------------+
/// 72      | vbe_control_info  |    (present if flags[11] is set)
/// 76      | vbe_mode_info     |
/// 80      | vbe_mode          |
/// 82      | vbe_interface_seg |
/// 84      | vbe_interface_off |
/// 86      | vbe_interface_len |
///         +-------------------+
/// 88      | framebuffer_addr  |    (present if flags[12] is set)
/// 96      | framebuffer_pitch |
/// 100     | framebuffer_width |
/// 104     | framebuffer_height|
/// 108     | framebuffer_bpp   |
/// 109     | framebuffer_type  |
/// 110-115 | color_info        |
///         +-------------------+
///```
///
#[derive(Debug, Copy, Clone)]
#[repr(C, packed)]
struct MultibootLegacyInfo {
    /// Indicate whether the below field exists.
    flags: u32,

    /// Physical memory low.
    mem_lower: u32,
    /// Physical memory high.
    mem_upper: u32,

    /// Indicates which BIOS disk device the boot loader loaded the OS image from.
    boot_device: u32,

    /// Command line passed to kernel.
    cmdline: u32,

    /// Modules count.
    mods_count: u32,
    /// The start address of modules list, each module structure format:
    /// ```text
    ///         +-------------------+
    /// 0       | mod_start         |
    /// 4       | mod_end           |
    ///         +-------------------+
    /// 8       | string            |
    ///         +-------------------+
    /// 12      | reserved (0)      |
    ///         +-------------------+
    /// ```
    mods_addr: u32,

    /// If flags[4] = 1, then the field starting at byte 28 are valid:
    /// ```text
    ///         +-------------------+
    /// 28      | tabsize           |
    /// 32      | strsize           |
    /// 36      | addr              |
    /// 40      | reserved (0)      |
    ///         +-------------------+
    /// ```
    /// These indicate where the symbol table from kernel image can be found.
    ///
    /// If flags[5] = 1, then the field starting at byte 28 are valid:
    /// ```text
    ///         +-------------------+
    /// 28      | num               |
    /// 32      | size              |
    /// 36      | addr              |
    /// 40      | shndx             |
    ///         +-------------------+
    /// ```
    /// These indicate where the section header table from an ELF kernel is,
    /// the size of each entry, number of entries, and the string table used as the index of names.
    symbols: [u8; 16],

    memory_map_len: u32,
    /// The start address of memory map list, each structure format:
    /// ```text
    ///         +-------------------+
    /// -4      | size              |
    ///         +-------------------+
    /// 0       | base_addr         |
    /// 8       | length            |
    /// 16      | type              |
    ///         +-------------------+
    /// ```
    memory_map_addr: u32,

    drives_length: u32,
    drives_addr: u32,

    config_table: u32,

    boot_loader_name: u32,

    apm_table: u32,

    vbe_table: VbeTable,

    framebuffer_table: FramebufferTable,
}

#[derive(Debug, Copy, Clone)]
#[repr(C, packed)]
struct VbeTable {
    control_info: u32,
    mode_info: u32,
    mode: u16,
    interface_seg: u16,
    interface_off: u16,
    interface_len: u16,
}

#[derive(Debug, Copy, Clone)]
#[repr(C, packed)]
struct FramebufferTable {
    addr: u64,
    pitch: u32,
    width: u32,
    height: u32,
    bpp: u8,
    typ: u8,
    color_info: [u8; 6],
}

#[derive(Debug, Copy, Clone)]
#[repr(C, packed)]
struct MemoryEntry {
    size: u32,
    base_addr: u64,
    length: u64,
    memory_type: MemoryAreaType,
}

// The entry point of kernel code, which should be defined by the package that
// uses jinux-frame.
extern "Rust" {
    fn jinux_main() -> !;
}

static MB1_INFO: Once<&'static MultibootLegacyInfo> = Once::new();

pub(super) unsafe fn multiboot_entry(boot_magic: u32, boot_params: u64) -> ! {
    assert_eq!(boot_magic, MULTIBOOT_ENTRY_MAGIC);
    MB1_INFO.call_once(|| &*(paddr_to_vaddr(boot_params as usize) as *const MultibootLegacyInfo));
    jinux_main();
}
