#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![forbid(unsafe_code)]
// #![feature(default_alloc_error_handler)]
extern crate kxos_frame;

use bootloader::{entry_point, BootInfo};
use core::panic::PanicInfo;
use kxos_frame::println;

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    // turn the screen gray
    // if let Some(framebuffer) = boot_info.framebuffer.as_mut() {
    //     for byte in framebuffer.buffer_mut() {
    //         *byte = 0x00;
    //     }
    // }
    kxos_frame::init(boot_info);
    println!("finish init kxos_frame");

    kxos_std::init();
    kxos_std::run_first_process();

    loop {}
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}
