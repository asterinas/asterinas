#![no_std]
#![no_main]

mod console;

use core::arch::global_asm;

global_asm!(include_str!("header.S"));

#[no_mangle]
pub extern "cdecl" fn _rust_setup_entry(boot_params_ptr: u32) -> ! {
    // safety: this init function is only called once
    unsafe { console::init() };
    println!("[setup] boot_params_ptr: {:#x}", boot_params_ptr);
    #[allow(clippy::empty_loop)]
    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
