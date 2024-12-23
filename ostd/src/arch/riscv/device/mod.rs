// SPDX-License-Identifier: MPL-2.0

//! Device-related APIs.
//! This module mainly contains the APIs that should exposed to the device driver like PCI, RTC

pub mod io_port;
pub(crate) mod plic;

use crate::{
    io_mem::IoMem,
    mm::page_prop::{CachePolicy, PageFlags},
};

pub(crate) fn init() {
    plic::init();
}

pub(crate) unsafe fn create_device_io_mem(starting_address: *const u8, size: usize) -> IoMem {
    IoMem::new(
        (starting_address as usize)..(starting_address as usize) + size,
        PageFlags::RW,
        CachePolicy::Uncacheable,
    )
}
