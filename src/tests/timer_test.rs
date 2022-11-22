#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(jinux_frame::test_runner)]
#![reexport_test_harness_main = "test_main"]
use bootloader::{entry_point, BootInfo};
use jinux_frame::timer::Timer;
extern crate alloc;
use alloc::sync::Arc;
use core::panic::PanicInfo;
use core::time::Duration;
use jinux_frame::println;

static mut TICK: usize = 0;

entry_point!(kernel_test_main);

fn kernel_test_main(boot_info: &'static mut BootInfo) -> ! {
    jinux_frame::init(boot_info);
    test_main();
    loop {}
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    jinux_frame::test_panic_handler(info)
}

#[test_case]
fn test_timer() {
    println!(
        "If you want to pass this test, you may need to enable the interrupt in jinux_frame/lib.rs"
    );
    println!("make sure the Timer irq number 32 handler won't panic");
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
