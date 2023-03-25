//! Device-related APIs.

pub mod cmos;
pub mod io_port;
pub mod pci;
pub mod serial;

/// Call after the memory allocator init
pub(crate) fn init() {
    serial::callback_init();
}
