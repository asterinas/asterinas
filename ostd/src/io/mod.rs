// SPDX-License-Identifier: MPL-2.0

//! Device I/O access and corresponding allocator.
//!
//! This module allows device drivers to access the device I/O they need
//! through _allocators_. There are two types of device I/O:
//!  - `IoMem` for memory I/O (MMIO).
//!  - `IoPort` for port I/O (PIO).

mod io_mem;

pub use self::io_mem::IoMem;
pub(crate) use self::io_mem::IoMemAllocatorBuilder;

/// Initializes the static allocator based on builder.
///
/// # Safety
///
/// User must ensure all the memory I/O regions that belong to the system device have been removed by calling the
/// `remove` function.
pub(crate) unsafe fn init(builder: IoMemAllocatorBuilder) {
    self::io_mem::init(builder);
    // TODO: IoPort initialization
}
