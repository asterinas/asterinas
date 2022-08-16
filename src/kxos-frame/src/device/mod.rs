//! Device-related APIs.

pub mod framebuffer;
mod io_port;
mod irq;

use bootloader::BootInfo;

pub use self::io_port::IoPort;
pub use self::irq::{InterruptInformation, IrqCallbackHandle, IrqLine};

pub fn init(boot_info: &'static mut BootInfo) {
    framebuffer::init(boot_info.framebuffer.as_mut().unwrap());
    irq::init();
}
