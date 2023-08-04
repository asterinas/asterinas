//! The boot module defines the entrypoints of Jinux and the corresponding
//! headers for different bootloaders.
//!
//! We currently support Multiboot2. The support for Linux Boot Protocol is
//! on its way.
//!
//! In this module, we use println! to print information on screen rather
//! than logging since the logger is not initialized here.
//!

pub mod multiboot2;
use self::multiboot2::init_global_boot_statics;

pub mod memory_region;
use self::memory_region::MemoryRegion;

#[cfg(feature = "intel_tdx")]
use crate::E820Entry;
use alloc::{string::String, vec::Vec};
#[cfg(feature = "intel_tdx")]
use memory_region::MemoryRegionType;
use spin::Once;

/// The boot crate can choose either providing the raw RSDP physical address or
/// providing the RSDT/XSDT physical address after parsing RSDP.
/// This is because bootloaders differ in such behaviors.
#[derive(Copy, Clone, Debug)]
pub enum BootloaderAcpiArg {
    /// Physical address of the RSDP.
    Rsdp(usize),
    /// Address of RSDT provided in RSDP v1.
    Rsdt(usize),
    /// Address of XSDT provided in RSDP v2+.
    Xsdt(usize),
}

/// The framebuffer arguments.
#[derive(Copy, Clone, Debug)]
pub struct BootloaderFramebufferArg {
    pub address: usize,
    pub width: usize,
    pub height: usize,
    /// Bits per pixel of the buffer.
    pub bpp: usize,
}

/// After initializing the boot module, the get_* functions could be called.
/// The initialization must be done after the heap is set and before physical
/// mappings are cancelled.
pub fn init() {
    init_global_boot_statics();
}

// The public get_* APIs.

static BOOTLOADER_NAME: Once<String> = Once::new();
/// Get the name and the version of the bootloader.
pub fn get_bootloader_name() -> String {
    BOOTLOADER_NAME.get().unwrap().clone()
}

static KERNEL_COMMANDLINE: Once<String> = Once::new();
/// Get the raw unparsed kernel commandline string.
pub fn get_kernel_commandline() -> String {
    KERNEL_COMMANDLINE.get().unwrap().clone()
}

static INITRAMFS: Once<&'static [u8]> = Once::new();
/// The slice of the bootloader-loaded init ram disk.
pub fn get_initramfs() -> &'static [u8] {
    INITRAMFS.get().unwrap()
}

static ACPI_RSDP: Once<BootloaderAcpiArg> = Once::new();
/// The ACPI RDSP/XSDT address.
pub fn get_acpi_rsdp() -> BootloaderAcpiArg {
    ACPI_RSDP.get().unwrap().clone()
}

static FRAMEBUFFER_INFO: Once<BootloaderFramebufferArg> = Once::new();
/// Framebuffer information.
pub fn get_framebuffer_info() -> BootloaderFramebufferArg {
    FRAMEBUFFER_INFO.get().unwrap().clone()
}

static MEMORY_REGIONS: Once<Vec<MemoryRegion>> = Once::new();
/// Get memory regions.
/// The returned usable memory regions are guarenteed to not overlap with other unusable ones.
#[cfg(not(feature = "intel_tdx"))]
pub fn get_memory_regions() -> Vec<MemoryRegion> {
    MEMORY_REGIONS.get().unwrap().clone()
}

#[cfg(feature = "intel_tdx")]
pub fn get_memory_regions(memory: [E820Entry; 128]) -> Vec<MemoryRegion> {
    let memory_regions: Vec<MemoryRegion> = memory
        .iter()
        .filter(|memory_region| memory_region.r#type == 1)
        .map(|memory_region| MemoryRegion {
            base: memory_region.addr as usize,
            len: memory_region.size as usize,
            typ: MemoryRegionType::Usable,
        })
        .collect();
    memory_regions
}
