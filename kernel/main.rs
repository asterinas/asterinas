#![no_std]
#![no_main]
// The `no_mangle`` attribute for the `jinux_main` entrypoint requires the removal of safety check.
// Please be aware that the kernel is not allowed to introduce any other unsafe operations.
// #![forbid(unsafe_code)]
extern crate jinux_frame;

use core::panic::PanicInfo;
use jinux_frame::println;

#[no_mangle]
pub fn jinux_main() -> ! {
    jinux_frame::init();
    println!("[kernel] finish init jinux_frame");
    component::init_all(component::parse_metadata!()).unwrap();
    jinux_std::init();
    jinux_std::run_first_process();
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    jinux_frame::panic_handler(info);
}
