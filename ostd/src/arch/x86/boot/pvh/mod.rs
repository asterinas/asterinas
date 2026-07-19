// SPDX-License-Identifier: MPL-2.0

//! The PVH boot protocol supporting module.
//!
//! PVH is the direct boot protocol implemented by most modern virtual machine
//! monitors (e.g., Cloud Hypervisor, Firecracker, QEMU). The VMM enters the
//! kernel in 32-bit protected mode with paging disabled and passes the
//! physical address of an [`HvmStartInfo`] structure in `EBX`. The entry point
//! itself is advertised through the PVH ELF note (see `note.S`).
//!
//! Reference: <https://xenbits.xen.org/docs/unstable/misc/pvh.html>

use core::arch::global_asm;

use crate::{
    boot::{
        BootloaderAcpiArg,
        memory_region::{MemoryRegion, MemoryRegionArray, MemoryRegionType},
    },
    mm::{Paddr, kspace::paddr_to_vaddr},
};

global_asm!(include_str!("note.S"));

/// The magic value of [`HvmStartInfo::magic`], which is "xEn3" in ASCII.
const HVM_START_MAGIC_VALUE: u32 = 0x336e_c578;

/// The start-of-day structure that a PVH-capable VMM passes to the guest.
#[repr(C)]
struct HvmStartInfo {
    magic: u32,
    version: u32,
    flags: u32,
    nr_modules: u32,
    modlist_paddr: u64,
    cmdline_paddr: u64,
    rsdp_paddr: u64,
    // The following fields are only present in version 1 and later.
    memmap_paddr: u64,
    memmap_entries: u32,
    _reserved: u32,
}

/// An entry of the module list pointed to by [`HvmStartInfo::modlist_paddr`].
#[repr(C)]
struct HvmModlistEntry {
    paddr: u64,
    size: u64,
    cmdline_paddr: u64,
    _reserved: u64,
}

/// An entry of the memory map pointed to by [`HvmStartInfo::memmap_paddr`].
#[repr(C)]
struct HvmMemmapTableEntry {
    addr: u64,
    size: u64,
    typ: u32,
    _reserved: u32,
}

/// Converts a PVH memory map entry type, which mirrors the E820 types.
fn parse_memory_region_type(typ: u32) -> MemoryRegionType {
    match typ {
        1 => MemoryRegionType::Usable,
        2 => MemoryRegionType::Reserved,
        3 => MemoryRegionType::Reclaimable,
        4 => MemoryRegionType::NonVolatileSleep,
        5 => MemoryRegionType::BadMemory,
        // All other memory regions are reserved.
        _ => MemoryRegionType::Reserved,
    }
}

fn parse_kernel_commandline(start_info: &HvmStartInfo) -> Option<&'static str> {
    if start_info.cmdline_paddr == 0 {
        return None;
    }

    let cmdline_ptr = paddr_to_vaddr(start_info.cmdline_paddr as Paddr);
    // SAFETY: The command line is a NUL-terminated string that is safe to read because of the
    // contract with the VMM, and it lives for `'static`.
    let cmdline = unsafe { core::ffi::CStr::from_ptr(cmdline_ptr as *const _) };

    cmdline.to_str().ok()
}

fn parse_initramfs(start_info: &HvmStartInfo) -> Option<&'static [u8]> {
    if start_info.nr_modules == 0 || start_info.modlist_paddr == 0 {
        return None;
    }

    let modlist_ptr = paddr_to_vaddr(start_info.modlist_paddr as Paddr);
    // SAFETY: The module list contains at least one entry (checked above) and is safe to read
    // because of the contract with the VMM.
    let module = unsafe { &*(modlist_ptr as *const HvmModlistEntry) };

    if module.paddr == 0 || module.size == 0 {
        return None;
    }

    let initramfs_ptr = paddr_to_vaddr(module.paddr as Paddr);
    // SAFETY: The initramfs is safe to read because of the contract with the VMM.
    let initramfs =
        unsafe { core::slice::from_raw_parts(initramfs_ptr as *const u8, module.size as usize) };

    Some(initramfs)
}

fn parse_acpi_arg(start_info: &HvmStartInfo) -> BootloaderAcpiArg {
    if start_info.rsdp_paddr == 0 {
        // The VMM did not provide the RSDP address, so fall back to scanning for it.
        BootloaderAcpiArg::ScanBios
    } else {
        BootloaderAcpiArg::Rsdp(
            start_info
                .rsdp_paddr
                .try_into()
                .expect("RSDP address overflowed!"),
        )
    }
}

fn parse_memory_regions(start_info: &HvmStartInfo) -> MemoryRegionArray {
    let mut regions = MemoryRegionArray::new();

    // The memory map is only available in version 1 and later.
    if start_info.version >= 1 && start_info.memmap_paddr != 0 {
        let memmap_ptr = paddr_to_vaddr(start_info.memmap_paddr as Paddr);
        // SAFETY: The memory map is safe to read because of the contract with the VMM.
        let memmap = unsafe {
            core::slice::from_raw_parts(
                memmap_ptr as *const HvmMemmapTableEntry,
                start_info.memmap_entries as usize,
            )
        };

        for entry in memmap {
            regions
                .push(MemoryRegion::new(
                    entry.addr.try_into().unwrap(),
                    entry.size.try_into().unwrap(),
                    parse_memory_region_type(entry.typ),
                ))
                .unwrap();
        }
    }

    // Add the kernel region since the VMM does not specify it.
    regions.push(MemoryRegion::kernel()).unwrap();

    // Add the initramfs region.
    if let Some(initramfs) = parse_initramfs(start_info) {
        regions.push(MemoryRegion::module(initramfs)).unwrap();
    }

    // Add the AP boot code region that will be copied into by the BSP.
    regions
        .push(super::smp::reclaimable_memory_region())
        .unwrap();

    // Add the region of the kernel cmdline since the VMM does not specify it.
    if let Some(kcmdline) = parse_kernel_commandline(start_info) {
        regions
            .push(MemoryRegion::module(kcmdline.as_bytes()))
            .unwrap();
    }

    regions.into_non_overlapping()
}

/// The entry point of the Rust code portion of Asterinas (with PVH parameters).
///
/// # Safety
///
/// - This function must be called only once at a proper timing in the BSP's boot assembly code.
/// - The caller must follow C calling conventions and put the right arguments in registers.
/// - If this function is called, entry points of other boot protocols must never be called.
// SAFETY: The name does not collide with other symbols.
#[unsafe(no_mangle)]
unsafe extern "sysv64" fn __pvh_entry(start_info_ptr: *const HvmStartInfo) -> ! {
    // SAFETY: We get the start-of-day structure from the VMM, so by contract the pointer is valid
    // and the underlying memory is initialized.
    let start_info = unsafe { &*start_info_ptr };
    assert_eq!({ start_info.magic }, HVM_START_MAGIC_VALUE);

    use crate::boot::{EARLY_INFO, EarlyBootInfo, start_kernel};

    EARLY_INFO.call_once(|| EarlyBootInfo {
        bootloader_name: "PVH-capable VMM",
        kernel_cmdline: parse_kernel_commandline(start_info).unwrap_or(""),
        initramfs: parse_initramfs(start_info),
        acpi_arg: parse_acpi_arg(start_info),
        framebuffer_arg: None,
        memory_regions: parse_memory_regions(start_info),
    });

    // SAFETY: The safety is guaranteed by the safety preconditions and the fact that we call it
    // once after setting up necessary resources.
    unsafe { start_kernel() };
}
