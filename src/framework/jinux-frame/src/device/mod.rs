//! Device-related APIs.

pub mod framebuffer;
pub mod io_port;
pub mod pci;
pub mod serial;

/// Call after the memory allocator init
pub(crate) fn init() {
    framebuffer::init();
    serial::callback_init();
}
