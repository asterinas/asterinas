#![no_std]
#![no_main]

use core::arch::global_asm;

global_asm!(include_str!("header.S"));

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
