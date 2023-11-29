#![no_std]
#![no_main]

use linux_boot_params::BootParams;

mod console;
mod loader;
mod x86;

use console::{print_hex, print_str};

/// The entrypoint of the trojan. The architecture-specific entrypoint will call this function.
fn trojan_entry(boot_params_ptr: usize) -> ! {
    // println!("[setup] bzImage loaded at {:#x}", x86::relocation::get_image_loaded_offset());
    unsafe {
        print_str("[setup] bzImage loaded at ");
        print_hex(x86::relocation::get_image_loaded_offset() as u64);
        print_str("\n");
    }

    // Safety: the boot_params_ptr is a valid pointer to be borrowed.
    let boot_params = unsafe { &*(boot_params_ptr as *const BootParams) };
    // Safety: the payload_offset and payload_length is valid.
    let payload = unsafe {
        let hdr = &boot_params.hdr;
        // The payload_offset field is not recorded in the relocation table, so we need to
        // calculate the loaded offset manually.
        let loaded_offset = x86::relocation::get_image_loaded_offset();
        let payload_offset = (loaded_offset + hdr.payload_offset as isize) as usize;
        let payload_length = hdr.payload_length as usize;
        core::slice::from_raw_parts_mut(payload_offset as *mut u8, payload_length as usize)
    };

    // println!("[setup] loading ELF payload at {:#x}", payload as *const _ as *const u8 as usize);
    unsafe {
        print_str("[setup] loading ELF payload at ");
        print_hex(payload as *const _ as *const u8 as u64);
        print_str("\n");
    }
    let entrypoint = loader::load_elf(payload);

    // println!("[setup] jumping to payload entrypoint at {:#x}", entrypoint);
    unsafe {
        print_str("[setup] jumping to payload entrypoint at ");
        print_hex(entrypoint as u64);
        print_str("\n");
    }

    // Safety: the entrypoint and the ptr is valid.
    unsafe { x86::call_aster_entrypoint(entrypoint.into(), boot_params_ptr.try_into().unwrap()) };
}
