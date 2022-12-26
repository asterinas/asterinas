#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![forbid(unsafe_code)]
#![test_runner(jinux_frame::test_runner)]
#![reexport_test_harness_main = "test_main"]
extern crate jinux_frame;

use bootloader::{entry_point, BootInfo};
use core::panic::PanicInfo;
use jinux_frame::println;

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    #[cfg(test)]
    test_main();
    jinux_frame::init(boot_info);
    println!("[kernel] finish init jinux_frame");

    jinux_std::init();
    jinux_std::run_first_process();
}
#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("[panic]:{:?}", info);
    jinux_frame::panic_handler();
    loop {}
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    jinux_frame::test_panic_handler(info);
}

#[test_case]
fn trivial_assertion() {
    assert_eq!(1, 1);
}
