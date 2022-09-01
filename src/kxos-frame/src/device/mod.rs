//! Device-related APIs.

pub mod framebuffer;
mod io_port;
pub mod serial;

pub use self::io_port::IoPort;

pub(crate) fn init(framebuffer: &'static mut bootloader::boot_info::FrameBuffer) {
    framebuffer::init(framebuffer);
}
