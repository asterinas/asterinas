// SPDX-License-Identifier: MPL-2.0

use alloc::string::ToString;

use aster_framebuffer::{CONSOLE_NAME, FRAMEBUFFER_CONSOLE};
use log::info;

pub fn init() {
    // print all the input device to make sure input crate will compile
    for device in aster_input::all_devices() {
        info!("Found Input device, name:{}", device.name());
    }

    if let Some(console) = FRAMEBUFFER_CONSOLE.get() {
        aster_console::register_device(CONSOLE_NAME.to_string(), console.clone());
    }
}
