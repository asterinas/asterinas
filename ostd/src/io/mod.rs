// SPDX-License-Identifier: MPL-2.0

//! Device I/O access and corresponding allocator.
//!
//! This module allows device drivers to access the device I/O they need
//! through _allocators_. There are two types of device I/O:
//!  - `IoMem` for memory I/O (MMIO).
//!  - `IoPort` for port I/O (PIO).

mod io_mem;

use cfg_if::cfg_if;

pub use self::io_mem::IoMem;
pub(crate) use self::io_mem::IoMemAllocatorBuilder;

cfg_if!(
    if #[cfg(target_arch = "x86_64")] {
        mod io_port;
        pub use io_port::IoPort;
        pub(crate) use self::io_port::{reserve_io_port_range, sensitive_io_port, RawIoPortRange};
    }
);

/// Initializes the static allocator based on builder.
///
/// # Safety
///
/// User must ensure that:
///
/// 1. All the memory that belong to the system device have been removed
///    by calling the `remove` function.
///
/// 2. All the port I/O regions belonging to the system device are defined
///    using the macros `sensitive_io_port` and `reserve_io_port_range`.
///
/// 3. `MAX_IO_PORT` defined in `crate::arch::io` is guaranteed not to
///    exceed the maximum value specified by architecture.
pub(crate) unsafe fn init(io_mem_builder: IoMemAllocatorBuilder) {
    // SAFETY: The safety is upheld by the caller.
    unsafe { self::io_mem::init(io_mem_builder) };

    // SAFETY: The safety is upheld by the caller.
    #[cfg(target_arch = "x86_64")]
    unsafe {
        self::io_port::init()
    };
}
