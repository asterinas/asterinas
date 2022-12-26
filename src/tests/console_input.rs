#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(jinux_frame::test_runner)]
#![reexport_test_harness_main = "test_main"]
use bootloader::{entry_point, BootInfo};
extern crate alloc;
use alloc::sync::Arc;
use core::panic::PanicInfo;
use jinux_frame::println;

static mut INPUT_VALUE: u8 = 0;

entry_point!(kernel_test_main);

fn kernel_test_main(boot_info: &'static mut BootInfo) -> ! {
    jinux_frame::init(boot_info);
    jinux_std::driver::console::init();
    test_main();
    loop {}
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    jinux_frame::test_panic_handler(info)
}

#[test_case]
fn test_input() {
    jinux_frame::enable_interrupts();
    println!("please input value into console to pass this test");
    jinux_std::driver::console::register_console_callback(Arc::new(input_callback));
    unsafe {
        while INPUT_VALUE == 0 {
            jinux_frame::hlt();
        }
        println!("input value:{}", INPUT_VALUE);
    }
}

pub fn input_callback(input: u8) {
    unsafe {
        INPUT_VALUE = input;
    }
}
