// SPDX-License-Identifier: MPL-2.0

//! Io memory access to the goldfish RTC device.

use spin::Once;

use crate::{
    arch::boot::DEVICE_TREE,
    io_mem::IoMem,
    mm::{CachePolicy, PageFlags},
};

/// [`IoMem`] of goldfish RTC, which will be used by `aster-time`.
pub static GOLDFISH_IO_MEM: Once<IoMem> = Once::new();

pub(crate) fn init() {
    let chosen = DEVICE_TREE.get().unwrap().find_node("/soc/rtc").unwrap();
    if let Some(compatible) = chosen.compatible()
        && compatible.all().any(|c| c == "google,goldfish-rtc")
    {
        let region = chosen.reg().unwrap().next().unwrap();
        let io_mem = unsafe {
            IoMem::new(
                (region.starting_address as usize)
                    ..(region.starting_address as usize) + region.size.unwrap(),
                PageFlags::RW,
                CachePolicy::Uncacheable,
            )
        };
        GOLDFISH_IO_MEM.call_once(|| io_mem);
    }
}
