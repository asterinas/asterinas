//! The architecture-independent boot module, which provides a universal interface
//! from the bootloader to the rest of the framework.
//!

use crate::arch::boot::init_boot_args;

pub mod kcmdline;
use kcmdline::KCmdlineArg;

pub mod memory_region;
use self::memory_region::MemoryRegion;

use alloc::{string::String, vec::Vec};
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

// Use a macro to simplify coding.
macro_rules! define_global_static_boot_arguments {
    ( $( $lower:ident, $upper:ident, $typ:ty; )* ) => {
        // Define statics and corresponding public get APIs.
        $(
            static $upper: Once<$typ> = Once::new();
            /// Macro generated public get API.
            pub fn $lower() -> &'static $typ {
                $upper.get().unwrap()
            }
        )*
        // Produce a init function call. The init function must
        // be defined in the `arch::boot` module conforming to this
        // definition.
        fn arch_init_boot_args() {
            init_boot_args( $( &$upper, )* );
        }
    };
}

// Define a series of static variable definitions and its APIs. The names in
// each line are:
//  1. The lowercase name of the variable, also the name of the get API;
//  2. the uppercase name of the variable;
//  3. the type of the variable.
define_global_static_boot_arguments!(
    bootloader_name, BOOTLOADER_NAME, String;
    kernel_cmdline, KERNEL_CMDLINE, KCmdlineArg;
    initramfs, INITRAMFS, &'static [u8];
    acpi_arg, ACPI_ARG, BootloaderAcpiArg;
    framebuffer_arg, FRAMEBUFFER_ARG, BootloaderFramebufferArg;
    memory_regions, MEMORY_REGIONS, Vec<MemoryRegion>;
);

/// After initializing the boot module, the get functions could be called.
/// The initialization must be done after the heap is set and before physical
/// mappings are cancelled.
pub fn init() {
    arch_init_boot_args();
}
