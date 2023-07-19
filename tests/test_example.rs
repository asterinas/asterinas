#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(jinux_frame::test_runner)]
#![reexport_test_harness_main = "test_main"]
use core::panic::PanicInfo;

#[no_mangle]
pub fn jinux_main() -> ! {
    jinux_frame::init();
    test_main();
    loop {}
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    jinux_frame::test_panic_handler(info)
}

#[test_case]
fn test_println() {
    jinux_frame::println!("test_println output");
}
