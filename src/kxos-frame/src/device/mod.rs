//! Device-related APIs.

pub mod framebuffer;
mod io_port;
mod irq;

pub use self::io_port::IoPort;
pub use self::irq::{InterruptInformation, IrqCallbackHandle, IrqLine};

pub fn init(framebuffer: &'static mut bootloader::boot_info::FrameBuffer) {
    framebuffer::init(framebuffer);
    irq::init();
}
