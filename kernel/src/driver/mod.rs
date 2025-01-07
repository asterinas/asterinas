// SPDX-License-Identifier: MPL-2.0

use alloc::{string::ToString, sync::Arc};

use aster_framebuffer::{FramebufferConsole, CONSOLE_NAME};
use log::info;

pub fn init() {
    // print all the input device to make sure input crate will compile
    for (name, _) in aster_input::all_devices() {
        info!("Found Input device, name:{}", name);
    }

    // register framebuffer console
    aster_console::register_device(
        CONSOLE_NAME.to_string(),
        Arc::new(FramebufferConsole::new()),
    );
}
