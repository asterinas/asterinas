//! Device-related APIs.

pub mod framebuffer;
mod io_port;
pub mod pci;
pub mod serial;
mod pic;

pub use pic::{TimerCallback, TIMER_FREQ};
pub(crate) use pic::{add_timeout_list,TICK};
pub use self::io_port::IoPort;

pub(crate) fn init(framebuffer: &'static mut bootloader::boot_info::FrameBuffer) {
    framebuffer::init(framebuffer);
    pic::init();
}
