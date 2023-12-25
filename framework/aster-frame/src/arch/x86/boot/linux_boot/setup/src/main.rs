#![no_std]
#![no_main]

mod boot_params;
mod console;
mod loader;

use core::arch::{asm, global_asm};

global_asm!(include_str!("header.S"));

unsafe fn call_aster_entrypoint(entrypoint: u32, boot_params_ptr: u32) -> ! {
    asm!("mov esi, {}", in(reg) boot_params_ptr);
    asm!("mov eax, {}", in(reg) entrypoint);
    asm!("jmp eax");

    unreachable!();
}

#[no_mangle]
pub extern "cdecl" fn _rust_setup_entry(boot_params_ptr: u32) -> ! {
    // Safety: this init function is only called once.
    unsafe { console::init() };
    println!("[setup] boot_params_ptr: {:#x}", boot_params_ptr);

    let payload_offset = unsafe { boot_params::get_payload_offset(boot_params_ptr) };
    let payload_length = unsafe { boot_params::get_payload_length(boot_params_ptr) };
    let payload = unsafe {
        core::slice::from_raw_parts_mut(payload_offset as *mut u8, payload_length as usize)
    };

    println!("[setup] loading ELF payload...");
    let entrypoint = loader::load_elf(payload);
    println!("[setup] entrypoint: {:#x}", entrypoint);

    // Safety: the entrypoint and the ptr is valid.
    unsafe { call_aster_entrypoint(entrypoint, boot_params_ptr) };
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("panic: {:?}", info);
    loop {}
}
