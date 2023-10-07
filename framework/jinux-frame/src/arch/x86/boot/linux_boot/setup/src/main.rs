#![no_std]
#![no_main]

mod console;

use core::arch::global_asm;

global_asm!(include_str!("header.S"));

#[no_mangle]
pub extern "C" fn _rust_setup_entry() -> ! {
    // safety: this init function is only called once
    unsafe { console::init() };
    println!("Hello, world!");
    #[allow(clippy::empty_loop)]
    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
