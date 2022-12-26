//! Device-related APIs.
pub mod framebuffer;

pub mod console;
mod io_port;
pub mod pci;

pub use self::io_port::IoPort;

/// first step to init device, call before the memory allocator init
pub(crate) fn first_init(framebuffer: &'static mut bootloader::boot_info::FrameBuffer) {
    framebuffer::init(framebuffer);
    console::init();
}

/// second step to init device, call after the memory allocator init
pub(crate) fn second_init() {
    console::register_console_input_callback(|trap| {});
}
