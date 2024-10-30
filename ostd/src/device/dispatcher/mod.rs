// SPDX-License-Identifier: MPL-2.0

//! Dispatchers that dispatch device I/O to device drivers.
//!
//! This module allows device drivers to access the device I/O they need
//! through _dispatchers_. Depending on the type of device I/O, there are two
//! types of dispatchers:
//!  - [`IoMemDispatcher`] for memory I/O (MMIO).
//!  - [`IoPortDispatcher`] for port I/O (PIO).
//!
//! [`IoMemDispatcher`]: io_mem::IoMemDispatcher
//! [`IoPortDispatcher`]: io_port::IoPortDispatcher

pub mod io_mem;
