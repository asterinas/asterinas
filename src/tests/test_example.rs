#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kxos_frame::test_runner)]
#![reexport_test_harness_main = "test_main"]
use bootloader::{entry_point, BootInfo};
use core::panic::PanicInfo;

entry_point!(kernel_test_main);

fn kernel_test_main(boot_info: &'static mut BootInfo) -> ! {
    kxos_frame::init(boot_info);
    test_main();
    loop {}
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    kxos_frame::test_panic_handler(info)
}

#[test_case]
fn test_println() {
    kxos_frame::println!("test_println output");
}
