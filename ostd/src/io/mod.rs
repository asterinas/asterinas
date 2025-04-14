// SPDX-License-Identifier: MPL-2.0

//! Device I/O access and corresponding allocator.
//!
//! This module allows device drivers to access the device I/O they need
//! through _allocators_. There are two types of device I/O:
//!  - `IoMem` for memory I/O (MMIO).
//!  - `IoPort` for port I/O (PIO).

mod io_mem;
mod io_port;

pub use self::{io_mem::IoMem, io_port::IoPort};
pub(crate) use self::{
    io_mem::IoMemAllocatorBuilder,
    io_port::{reserve_io_port_range, sensitive_io_port, IoPortAllocatorBuilder, RawIoPortRange},
};

/// Initializes the static allocator based on builder.
///
/// # Safety
///
/// User must ensure all the memory and port I/O regions that belong to the system device
/// have been removed by calling the corresponding `remove` function.
pub(crate) unsafe fn init(
    io_mem_builder: IoMemAllocatorBuilder,
    io_port_builder: IoPortAllocatorBuilder,
) {
    self::io_mem::init(io_mem_builder);
    self::io_port::init(io_port_builder);
}
