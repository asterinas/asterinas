#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
// The no_mangle macro need to remove the `forbid(unsafe_code)` macro. The bootloader needs the _start function
// to be no mangle so that it can jump into the entry point.
// #![forbid(unsafe_code)]
#![test_runner(jinux_frame::test_runner)]
#![reexport_test_harness_main = "test_main"]
extern crate jinux_frame;

use core::panic::PanicInfo;
use jinux_frame::println;
use limine::LimineBootInfoRequest;

static BOOTLOADER_INFO: LimineBootInfoRequest = LimineBootInfoRequest::new(0);

#[no_mangle]
pub extern "C" fn _start() -> ! {
    #[cfg(test)]
    test_main();
    jinux_frame::init();
    println!("[kernel] finish init jinux_frame");
    component::init_all(component::parse_metadata!()).unwrap();
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
