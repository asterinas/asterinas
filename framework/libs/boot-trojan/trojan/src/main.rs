#![no_std]
#![no_main]

use linux_boot_params::BootParams;

mod arch;
mod console;
mod loader;

use console::{print, print_hex};

/// The entrypoint of the trojan. The architecture-specific entrypoint will call this function.
///
/// The loaded address of the CODE32_START should be passed in as `loaded_base`, since the trojan
/// may be loaded at any address, and offsets in the header are not position-independent.
fn trojan_entry(loaded_base: usize, boot_params_ptr: usize) -> ! {
    // Safety: this init function is only called once.
    unsafe { console::init() };
    unsafe {
        print("[setup] bzImage loaded at ");
        print_hex(loaded_base);
        print("\n");
    }

    // Safety: the boot_params_ptr is a valid pointer to be borrowed.
    let boot_params = unsafe { &*(boot_params_ptr as *const BootParams) };
    let hdr = &boot_params.hdr;
    let payload_offset = loaded_base + hdr.payload_offset as usize;
    let payload_length = hdr.payload_length as usize;
    let payload = unsafe {
        core::slice::from_raw_parts_mut(payload_offset as *mut u8, payload_length as usize)
    };

    unsafe {
        print("[setup] loading ELF payload at ");
        print_hex(payload_offset);
        print("...\n");
    }
    let entrypoint = loader::load_elf(payload);

    unsafe {
        print("[setup] jumping to payload entrypoint at ");
        print_hex(entrypoint as usize);
        print("...\n");
    }
    // Safety: the entrypoint and the ptr is valid.
    unsafe { arch::call_aster_entrypoint(entrypoint.into(), boot_params_ptr.try_into().unwrap()) };
}
