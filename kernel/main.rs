#![no_std]
#![no_main]
// The no_mangle macro need to remove the `forbid(unsafe_code)` macro. The bootloader needs the jinux_main function
// to be no mangle so that it can jump into the entry point.
// #![forbid(unsafe_code)]
#![feature(custom_test_frameworks)]
#![test_runner(jinux_frame::test::test_runner)]

extern crate jinux_frame;

use jinux_frame::println;

#[no_mangle]
pub fn jinux_main() {
    jinux_frame::init();
    println!("[kernel] finish init jinux_frame");
    component::init_all(component::parse_metadata!()).unwrap();
    jinux_std::init();
    #[cfg(not(test))]
    jinux_std::run_first_process();
}
