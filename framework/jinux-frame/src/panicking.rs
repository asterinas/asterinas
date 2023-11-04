//! Panic support in Jinux Frame.

use alloc::boxed::Box;
use alloc::string::{String, ToString};

use crate::arch::qemu::{exit_qemu, QemuExitCode};
use crate::println;

#[derive(Clone, Debug)]
pub struct PanicInfo {
    pub message: String,
    pub file: String,
    pub line: usize,
    pub col: usize,
}

impl core::fmt::Display for PanicInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        writeln!(f, "Panicked at {}:{}:{}", self.file, self.line, self.col)?;
        writeln!(f, "{}", self.message)
    }
}

#[panic_handler]
pub fn panic_handler(info: &core::panic::PanicInfo) -> ! {
    let throw_info = PanicInfo {
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
