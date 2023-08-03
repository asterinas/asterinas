//! The boot module defines the entrypoints of Jinux and the corresponding
//! headers for different bootloaders.
//!
//! We currently support Multiboot2. The support for Linux Boot Protocol is
//! on its way.
//!

mod multiboot2;
pub use self::multiboot2::init_boot_args;
