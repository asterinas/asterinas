// SPDX-License-Identifier: MPL-2.0

//! Device I/O access and corresponding allocator.
//!
//! This module allows device drivers to access the device I/O they need
//! through _allocators_. Depending on the type of device I/O, there are two
//! types of allocators:
//!  - [`IoMemAllocator`] for memory I/O (MMIO).
//!  - [`IoPortAllocator`] for port I/O (PIO).
//!
//! [`IoMemAllocator`]: io_mem::allocator::IoMemAllocator
//! [`IoPortAllocator`]: io_port::allocator::IoPortAllocator

pub mod io_mem;
