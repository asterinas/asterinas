// SPDX-License-Identifier: MPL-2.0

//! The x86 boot module defines the entrypoints of Asterinas and
//! the corresponding headers for different x86 boot protocols.
//!
//! We directly support
//!
//!  - Multiboot
//!  - Multiboot2
//!  - Linux x86 Boot Protocol
//!  - PVH
//!
//! without any additional configurations.
//!
//! Asterinas differentiates the boot protocol by the entry point
//! chosen by the boot loader. In each entry point function,
//! the universal callback registration method from
//! `crate::boot` will be called. Thus the initialization of
//! boot information is transparent for the upper level kernel.
//!

mod linux_boot;
mod multiboot;
mod multiboot2;
mod pvh;

pub(crate) mod smp;

use core::{arch::global_asm, num::NonZeroUsize};

use acpi::rsdp::Rsdp;

use crate::{
    arch::kernel::acpi::AcpiMemoryHandler,
    boot::{
        BootloaderAcpiArg, BootloaderFramebufferArg, EarlyBootInfo,
        memory_region::{MemoryRegion, MemoryRegionArray, MemoryRegionType},
    },
};

global_asm!(
    include_str!("bsp_boot.S"),
    KCODE64 = const super::trap::gdt::KCODE64,
    KDATA = const super::trap::gdt::KDATA,
    KCODE32 = const super::trap::gdt::KCODE32,
);
global_asm!(include_str!("ap_boot.S"));

/// A bootloader-provided structure that can be converted into [`EarlyBootInfo`].
pub(super) trait ToEarlyBootInfo {
    /// The name of the bootloader.
    fn bootloader_name(&self) -> &'static str;

    /// The kernel command line, if provided.
    fn kernel_commandline(&self) -> Option<&'static str>;

    /// The initramfs bytes, if provided.
    fn initramfs(&self) -> Option<&'static [u8]>;

    /// The ACPI information provided by the bootloader.
    fn acpi_arg(&self) -> BootloaderAcpiArg;

    /// The framebuffer information, if provided.
    fn framebuffer_arg(&self) -> Option<BootloaderFramebufferArg>;

    /// Builds the bootloader-specific memory regions, appends the common
    /// regions, and resolves the overlaps.
    fn memory_regions(
        &self,
        initramfs: Option<&'static [u8]>,
        kernel_cmdline: Option<&'static str>,
        framebuffer_arg: Option<BootloaderFramebufferArg>,
    ) -> MemoryRegionArray;

    /// Converts the bootloader-provided information into [`EarlyBootInfo`].
    fn to_early_boot_info(&self) -> EarlyBootInfo {
        let kernel_cmdline = self.kernel_commandline();
        let initramfs = self.initramfs();
        let framebuffer_arg = self.framebuffer_arg();

        EarlyBootInfo {
            bootloader_name: self.bootloader_name(),
            kernel_cmdline: kernel_cmdline.unwrap_or(""),
            initramfs,
            acpi_arg: self.acpi_arg(),
            framebuffer_arg,
            memory_regions: self.memory_regions(initramfs, kernel_cmdline, framebuffer_arg),
        }
    }
}

/// Appends the memory regions common to all boot protocols and resolves the
/// overlapping regions.
pub(super) fn finish_memory_regions(
    mut regions: MemoryRegionArray,
    framebuffer_arg: Option<BootloaderFramebufferArg>,
    initramfs: Option<&'static [u8]>,
    kernel_cmdline: Option<&'static str>,
) -> MemoryRegionArray {
    if let Some(fb) = framebuffer_arg {
        regions.push(MemoryRegion::framebuffer(&fb)).unwrap();
    }

    regions.push(MemoryRegion::kernel()).unwrap();

    if let Some(initramfs) = initramfs {
        regions.push(MemoryRegion::module(initramfs)).unwrap();
    }

    regions.push(smp::reclaimable_memory_region()).unwrap();

    if let Some(kcmdline) = kernel_cmdline {
        regions
            .push(MemoryRegion::module(kcmdline.as_bytes()))
            .unwrap();
    }

    regions.into_non_overlapping()
}

/// Finds the physical address of the ACPI root table.
pub(super) fn find_acpi_root_table_address() -> Option<NonZeroUsize> {
    // Some boot paths do not provide the RSDP directly and are
    // BIOS-compatible: Multiboot v1 has no standard EFI System Table field,
    // and the PVH entry path under QEMU goes through SeaBIOS (the pvh.bin
    // option ROM may leave `rsdp_paddr` zero). So we use the BIOS RSDP scan
    // as the legacy fallback.
    //
    // SAFETY: These entry paths are treated as BIOS-compatible.
    let Ok(rsdp) = (unsafe { Rsdp::search_for_on_bios(AcpiMemoryHandler {}) }) else {
        return None;
    };

    if rsdp.revision() == 0 {
        NonZeroUsize::new(rsdp.rsdt_address() as usize)
    } else {
        NonZeroUsize::new(rsdp.xsdt_address() as usize)
    }
}

/// Promotes a reserved region containing the ACPI root table to `Reclaimable`.
pub(super) fn effective_region_type(
    typ: MemoryRegionType,
    base: usize,
    len: usize,
    acpi_root_table_address: Option<NonZeroUsize>,
) -> MemoryRegionType {
    if typ == MemoryRegionType::Reserved
        && acpi_root_table_address.is_some_and(|addr| (base..(base + len)).contains(&addr.get()))
    {
        MemoryRegionType::Reclaimable
    } else {
        typ
    }
}
