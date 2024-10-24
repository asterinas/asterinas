// SPDX-License-Identifier: MPL-2.0

//! The Device IO Distributor module allows users to access the required device IO.
//! OSTD categorizes device IO into 'IoMem' and 'IoPort', representing MMIO
//! and PIO access, respectively.
//!

pub mod io_mem;

pub(crate) fn init() {
    io_mem::init();
}
