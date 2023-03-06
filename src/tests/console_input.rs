#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(jinux_frame::test_runner)]
#![reexport_test_harness_main = "test_main"]
extern crate alloc;
use core::panic::PanicInfo;
use jinux_frame::println;

static mut INPUT_VALUE: u8 = 0;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    jinux_frame::init();
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
    jinux_std::driver::tty::register_serial_input_callback(input_callback);
    unsafe {
        while INPUT_VALUE == 0 {
            jinux_frame::hlt();
        }
        // println!("input value:{}", INPUT_VALUE);
    }
}

pub fn input_callback(input: u8) {
    println!("input value:{}", input);
    unsafe {
        INPUT_VALUE = input;
    }
}
