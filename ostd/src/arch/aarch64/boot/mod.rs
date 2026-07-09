// SPDX-License-Identifier: MPL-2.0

//! The AArch64 boot module defines the entrypoints of Asterinas.

pub(crate) mod smp;

use core::arch::global_asm;

use fdt::Fdt;
use spin::Once;

use crate::{
    boot::{
        BootloaderAcpiArg, BootloaderFramebufferArg,
        memory_region::{MemoryRegion, MemoryRegionArray, MemoryRegionType},
    },
    mm::paddr_to_vaddr,
};

global_asm!(include_str!("bsp_boot.S"));

/// The Flattened Device Tree of the platform.
pub static DEVICE_TREE: Once<Fdt> = Once::new();

fn parse_bootloader_name() -> &'static str {
    "Unknown"
}

fn parse_kernel_commandline() -> &'static str {
    DEVICE_TREE.get().unwrap().chosen().bootargs().unwrap_or("")
}

fn parse_initramfs() -> Option<&'static [u8]> {
    let (start, end) = parse_initramfs_range()?;

    let base_va = paddr_to_vaddr(start);
    let length = end - start;
    Some(unsafe { core::slice::from_raw_parts(base_va as *const u8, length) })
}

fn parse_acpi_arg() -> BootloaderAcpiArg {
    // AArch64 `virt` provides hardware description via the device tree; ACPI is
    // not consumed by OSTD yet.
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

    if let Some(node) = DEVICE_TREE.get().unwrap().find_node("/reserved-memory") {
        for child in node.children() {
            if let Some(reg_iter) = child.reg() {
                for region in reg_iter {
                    regions
                        .push(MemoryRegion::new(
                            region.starting_address as usize,
                            region.size.unwrap(),
                            MemoryRegionType::Reserved,
                        ))
                        .unwrap();
                }
            }
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

/// The FDT magic number (`0xd00d_feed`), stored big-endian in the header.
const FDT_MAGIC: u32 = 0xd00d_feed;

/// The base of RAM on the QEMU `virt` machine, where the generated DTB is
/// placed when booting an ELF image directly (the boot registers are not set in
/// that case, so `x0` cannot be relied upon).
const QEMU_VIRT_RAM_BASE: usize = 0x4000_0000;

/// Locates the Flattened Device Tree.
///
/// Uses the bootloader-provided physical address when it points at a valid FDT,
/// otherwise falls back to the base of RAM.
fn find_device_tree(hint_paddr: usize) -> usize {
    let has_fdt_magic = |paddr: usize| -> bool {
        let ptr = paddr_to_vaddr(paddr) as *const u32;
        // SAFETY: The linear mapping covers physical RAM; reading one word to
        // check the magic is safe.
        let word = unsafe { core::ptr::read_volatile(ptr) };
        u32::from_be(word) == FDT_MAGIC
    };

    if hint_paddr != 0 && has_fdt_magic(hint_paddr) {
        return hint_paddr;
    }
    if has_fdt_magic(QEMU_VIRT_RAM_BASE) {
        return QEMU_VIRT_RAM_BASE;
    }
    panic!("could not locate a Flattened Device Tree (x0 = {hint_paddr:#x})");
}

fn parse_initramfs_range() -> Option<(usize, usize)> {
    let chosen = DEVICE_TREE.get().unwrap().find_node("/chosen").unwrap();

    // The preferred source is the standard `linux,initrd-start`/`-end` pair.
    // QEMU only populates these when booting a Linux `Image`; for a directly
    // booted ELF it does not, so we also accept an `initrd=<paddr>,<size>`
    // token on the kernel command line (paired with QEMU `-device loader`).
    if let (Some(start), Some(end)) = (
        chosen.property("linux,initrd-start"),
        chosen.property("linux,initrd-end"),
    ) && let (Some(s), Some(e)) = (read_dtb_addr(start.value), read_dtb_addr(end.value))
    {
        return Some((s, e));
    }

    let cmdline = DEVICE_TREE
        .get()
        .unwrap()
        .chosen()
        .bootargs()
        .unwrap_or("");
    parse_initrd_from_cmdline(cmdline)
}

/// Parses an `initrd=<paddr>,<size>` token from the kernel command line, where
/// both values are hexadecimal (`0x`-prefixed) or decimal.
fn parse_initrd_from_cmdline(cmdline: &str) -> Option<(usize, usize)> {
    // The command line from the device tree may be NUL-terminated, so trim NULs
    // and whitespace from each token before parsing.
    fn trim(s: &str) -> &str {
        s.trim_matches(|c: char| c == '\0' || c.is_whitespace())
    }
    let token = cmdline
        .split(|c: char| c.is_whitespace() || c == '\0')
        .find_map(|t| t.strip_prefix("initrd="))?;
    let (start_str, size_str) = token.split_once(',')?;
    let start = parse_int(trim(start_str))?;
    let size = parse_int(trim(size_str))?;
    Some((start, start + size))
}

fn parse_int(s: &str) -> Option<usize> {
    if let Some(hex) = s.strip_prefix("0x") {
        usize::from_str_radix(hex, 16).ok()
    } else {
        s.parse().ok()
    }
}

/// Reads a device-tree address value that may be encoded as either a 32-bit or
/// a 64-bit big-endian integer.
fn read_dtb_addr(value: &[u8]) -> Option<usize> {
    match value.len() {
        4 => Some(u32::from_be_bytes(value.try_into().ok()?) as usize),
        8 => Some(u64::from_be_bytes(value.try_into().ok()?) as usize),
        _ => None,
    }
}

/// The entry point of the Rust code portion of Asterinas.
///
/// # Safety
///
/// - This function must be called only once at a proper timing in the BSP's
///   boot assembly code.
/// - The caller must follow C calling conventions and put the DTB physical
///   address in the first argument register.
// SAFETY: The name does not collide with other symbols.
#[unsafe(no_mangle)]
unsafe extern "C" fn aarch64_boot(device_tree_paddr: usize) -> ! {
    // Install the exception vectors early so that any fault during boot (e.g.
    // while building or activating the kernel page table) is reported as a
    // panic rather than hanging on the firmware's default vector.
    // SAFETY: Called once, very early on the BSP, before any trap can occur.
    unsafe { super::trap::init_on_cpu() };

    let dtb_paddr = find_device_tree(device_tree_paddr);
    let device_tree_ptr = paddr_to_vaddr(dtb_paddr) as *const u8;
    // SAFETY: `find_device_tree` verified the FDT magic at this address, which is
    // mapped by the boot page tables via the linear mapping.
    let fdt = unsafe { Fdt::from_ptr(device_tree_ptr).unwrap() };
    DEVICE_TREE.call_once(|| fdt);

    use crate::boot::{EARLY_INFO, EarlyBootInfo, start_kernel};

    EARLY_INFO.call_once(|| EarlyBootInfo {
        bootloader_name: parse_bootloader_name(),
        kernel_cmdline: parse_kernel_commandline(),
        initramfs: parse_initramfs(),
        acpi_arg: parse_acpi_arg(),
        framebuffer_arg: parse_framebuffer_info(),
        memory_regions: parse_memory_regions(),
    });

    // SAFETY: The safety is guaranteed by the safety preconditions and the fact
    // that we call it once after setting up necessary resources.
    unsafe { start_kernel() };
}
