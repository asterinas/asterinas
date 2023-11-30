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
    let loaded_offset = x86::relocation::get_image_loaded_offset();
    let payload_offset = (loaded_offset + hdr.payload_offset as isize) as usize;
    let payload_length = hdr.payload_length as usize;
    // Safety: the payload_offset and payload_length is valid if we assume that the
    // boot_params struct is correct.
    unsafe { core::slice::from_raw_parts_mut(payload_offset as *mut u8, payload_length as usize) }
}
