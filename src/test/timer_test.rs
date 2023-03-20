#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(jinux_frame::test_runner)]
#![reexport_test_harness_main = "test_main"]
use jinux_frame::timer::Timer;
extern crate alloc;
use alloc::sync::Arc;
use core::panic::PanicInfo;
use core::time::Duration;
use jinux_frame::println;

static mut TICK: usize = 0;

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
fn test_timer() {
    jinux_frame::enable_interrupts();
    unsafe {
        let timer = Timer::new(timer_callback).unwrap();
        timer.set(Duration::from_secs(1));
        while TICK < 5 {}
    }
}

pub fn timer_callback(timer: Arc<Timer>) {
    unsafe {
        TICK += 1;
        println!("TICK:{}", TICK);
        timer.set(Duration::from_secs(1));
    }
}
