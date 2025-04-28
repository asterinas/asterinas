// SPDX-License-Identifier: MPL-2.0

//! Goldfish RTC access

use spin::Once;

use crate::{arch::boot::DEVICE_TREE, io::IoMem};

/// [`IoMem`] of goldfish RTC, which will be used by `aster-time`.
pub static GOLDFISH_IO_MEM: Once<IoMem> = Once::new();

/// Initialize the goldfish RTC device.
pub fn init() {
    let chosen = DEVICE_TREE.get().unwrap().find_node("/soc/rtc").unwrap();
    if let Some(compatible) = chosen.compatible()
        && compatible.all().any(|c| c == "google,goldfish-rtc")
    {
        let region = chosen.reg().unwrap().next().unwrap();
        let io_mem = IoMem::acquire(
            region.starting_address as usize
                ..region.starting_address as usize + region.size.unwrap(),
        )
        .unwrap();
        GOLDFISH_IO_MEM.call_once(|| io_mem);
    }
}
