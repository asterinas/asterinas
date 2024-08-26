// SPDX-License-Identifier: MPL-2.0

//! The x86 boot module defines the entrypoints of Asterinas and
//! the corresponding headers for different x86 boot protocols.
//!
//! We directly support
//!
//!  - Multiboot
//!  - Multiboot2
//!  - Linux x86 Boot Protocol
//!
//! without any additional configurations.
//!
//! Asterinas diffrentiates the boot protocol by the entry point
//! chosen by the boot loader. In each entry point function,
//! the universal callback registration method from
//! `crate::boot` will be called. Thus the initialization of
//! boot information is transparent for the upper level kernel.
//!

mod linux_boot;
mod multiboot;
mod multiboot2;

pub mod smp;

use core::arch::global_asm;

global_asm!(include_str!("bsp_boot.S"));
global_asm!(include_str!("ap_boot.S"));
