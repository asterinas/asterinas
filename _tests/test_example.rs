#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(jinux_frame::test::test_runner)]

#[no_mangle]
pub fn jinux_main() {
    jinux_frame::init();
}

#[test_case]
fn test_println() {
    jinux_frame::println!("test_println output");
}
