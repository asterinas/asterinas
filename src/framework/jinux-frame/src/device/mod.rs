//! Device-related APIs.
pub mod framebuffer;

mod io_port;
pub mod pci;
pub mod serial;

pub use self::io_port::IoPort;

/// Call after the memory allocator init
pub(crate) fn init() {
    framebuffer::init();
    serial::callback_init();
}
