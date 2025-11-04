// SPDX-License-Identifier: MPL-2.0

use alloc::string::ToString;

use aster_framebuffer::{CONSOLE_NAME, FRAMEBUFFER_CONSOLE};
use log::info;

pub fn init() {
    for device in aster_input::all_devices() {
        info!("Found an input device, name:{}", device.name());
    }

    // FIXME: Currently, we have to do this manually to ensure the crates containing the input
    // devices are linked and their `#[init_component]` hooks can run to register the devices with
    // the input core. We should find a way to avoid this in the future.
    #[expect(unused_imports)]
    use aster_i8042::*;

    if let Some(console) = FRAMEBUFFER_CONSOLE.get() {
        aster_console::register_device(CONSOLE_NAME.to_string(), console.clone());
    }
}
