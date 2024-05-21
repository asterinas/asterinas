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

use linux_boot_params::BootParams;

mod console;
mod loader;

// Unfortunately, the entrypoint is not defined here in the main.rs file.
// See the exported functions in the x86 module for details.
mod x86;

fn get_payload(boot_params: &BootParams) -> &'static [u8] {
    let hdr = &boot_params.hdr;
    // The payload_offset field is not recorded in the relocation table, so we need to
    // calculate the loaded offset manually.
    let loaded_offset = x86::get_image_loaded_offset();
    let payload_offset = (loaded_offset + hdr.payload_offset as isize) as usize;
    let payload_length = hdr.payload_length as usize;
    // SAFETY: the payload_offset and payload_length is valid if we assume that the
    // boot_params struct is correct.
    unsafe { core::slice::from_raw_parts_mut(payload_offset as *mut u8, payload_length) }
}
