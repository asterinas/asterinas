#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(jinux_frame::test_runner)]
#![reexport_test_harness_main = "test_main"]
extern crate alloc;
use core::panic::PanicInfo;

#[no_mangle]
pub fn jinux_main() -> ! {
    jinux_frame::init();
    component::init_all(component::parse_metadata!()).unwrap();
    test_main();
    loop {}
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    jinux_frame::test_panic_handler(info)
}

#[test_case]
fn test_framebuffer() {
    for _i in 0..30 {
        jinux_framebuffer::println!("test_println!");
    }
}
