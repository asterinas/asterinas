//! Panic support.

use alloc::{boxed::Box, string::ToString};

use crate::arch::qemu::{exit_qemu, QemuExitCode};
use crate::println;

#[panic_handler]
fn panic_handler(info: &core::panic::PanicInfo) -> ! {
    let throw_info = ktest::PanicInfo {
        message: info.message().unwrap().to_string(),
        file: info.location().unwrap().file().to_string(),
        line: info.location().unwrap().line() as usize,
        col: info.location().unwrap().column() as usize,
    };
    // Throw an exception and expecting it to be caught.
    unwinding::panic::begin_panic(Box::new(throw_info.clone()));
    // If the exception is not caught (e.g. by ktest), then print the information
    // and exit failed using the debug device.
    println!("[uncaught panic] {}", info);
    exit_qemu(QemuExitCode::Failed);
}
