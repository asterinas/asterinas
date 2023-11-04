#![no_std]
#![no_main]
// The no_mangle macro need to remove the `forbid(unsafe_code)` macro. The bootloader needs the _start function
// to be no mangle so that it can jump into the entry point.
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
    use jinux_frame::{exit_qemu, QemuExitCode};

    println!("[panic]:{:#?}", info);
    jinux_frame::panic_handler();
    exit_qemu(QemuExitCode::Failed);
}
