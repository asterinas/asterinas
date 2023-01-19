//! Device-related APIs.
pub mod framebuffer;

mod io_port;
pub mod pci;
pub mod serial;

pub use self::io_port::IoPort;

/// first step to init device, call before the memory allocator init
pub(crate) fn first_init(framebuffer: &'static mut bootloader::boot_info::FrameBuffer) {
    framebuffer::init(framebuffer);
    serial::init();
}

/// second step to init device, call after the memory allocator init
pub(crate) fn second_init() {
    serial::register_serial_input_irq_handler(|trap| {});
}
