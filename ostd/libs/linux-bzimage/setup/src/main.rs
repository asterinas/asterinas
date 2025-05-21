// SPDX-License-Identifier: MPL-2.0

//! The linux bzImage setup binary.
//!
//! With respect to the format of the bzImage, we design our bzImage setup in the similar
//! role as the setup code in the linux kernel. The setup code is responsible for
//! initializing the machine state, decompressing and loading the kernel image into memory.
//! So does our bzImage setup.
//!
//! The bzImage setup code is concatenated to the bzImage, and it contains both the linux
//! boot header and the PE/COFF header to be a valid UEFI image. The setup also supports
//! the legacy 32 bit boot protocol, but the support for the legacy boot protocol does not
//! co-exist with the UEFI boot protocol. Users can choose either one of them. By specifying
//! the target as `x86_64-unknown-none` it supports UEFI protocols. And if the target is
//! `x86_64-i386_pm-none` it supports the legacy boot protocol.
//!
//! The building process of the bzImage and the generation of the PE/COFF header is done
//! by the linux-bzimage-builder crate. And the code of the setup is in this crate.
//! You should compile this crate using the functions provided in the builder.
//!

#![no_std]
#![no_main]
#![feature(maybe_uninit_fill)]
#![feature(maybe_uninit_slice)]
#![feature(maybe_uninit_write_slice)]

mod console;
mod loader;
mod sync;

// The entry points are defined in `x86/*/setup.S`.
mod x86;
