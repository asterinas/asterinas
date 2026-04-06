// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_console::{AnyConsoleDevice, ConsoleCallback};

use crate::device::tty::vt::manager::{VtConsole, default_vt};

const DEFAULT_VT_CONSOLE_NAME: &str = "Default-VT-Console";

/// A console device that forwards output to the default VT console.
#[derive(Debug)]
struct DefaultVtConsoleDevice;

impl DefaultVtConsoleDevice {
    fn vt_console(&self) -> &VtConsole {
        // Keep kernel logs on the default VT console so log visibility is
        // predictable for debugging.
        default_vt().driver().vt_console()
    }
}

impl AnyConsoleDevice for DefaultVtConsoleDevice {
    fn send(&self, buf: &[u8]) {
        self.vt_console().send(buf);
    }

    fn register_callback(&self, _callback: &'static ConsoleCallback) {}
}

pub(super) fn init_in_first_process() {
    aster_console::register_device(
        DEFAULT_VT_CONSOLE_NAME.into(),
        Arc::new(DefaultVtConsoleDevice),
    );
}
