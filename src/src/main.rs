#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![forbid(unsafe_code)]
#![test_runner(kxos_frame::test_runner)]
#![reexport_test_harness_main = "test_main"]
extern crate kxos_frame;

use bootloader::{entry_point, BootInfo};
use core::panic::PanicInfo;
use kxos_frame::println;

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    #[cfg(test)]
    test_main();
    kxos_frame::init(boot_info);
    println!("[kernel] finish init kxos_frame");

    kxos_std::init();
    kxos_std::run_first_process();

    loop {}
}
#[cfg(not(test))]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("[panic]:{:?}", info);
    loop {}
}

#[cfg(test)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    kxos_frame::test_panic_handler(info);
}

#[test_case]
fn trivial_assertion() {
    assert_eq!(1, 1);
}
