// SPDX-License-Identifier: MPL-2.0

use aster_console::{
    font::BitmapFont,
    mode::{ConsoleMode, KeyboardMode},
    AnyConsoleDevice, ConsoleCallback, ConsoleSetFontError,
};

/// A dummy console device.
///
/// This is used when no framebuffer is available. All operations are no-ops.
#[derive(Debug)]
pub struct DummyFramebufferConsole;

impl AnyConsoleDevice for DummyFramebufferConsole {
    fn send(&self, _buf: &[u8]) {}

    fn register_callback(&self, _callback: &'static ConsoleCallback) {}

    fn set_font(&self, _font: BitmapFont) -> Result<(), ConsoleSetFontError> {
        Err(ConsoleSetFontError::InappropriateDevice)
    }

    fn set_mode(&self, _mode: ConsoleMode) -> bool {
        false
    }

    fn mode(&self) -> Option<ConsoleMode> {
        None
    }

    fn set_keyboard_mode(&self, _mode: KeyboardMode) -> bool {
        false
    }

    fn keyboard_mode(&self) -> Option<KeyboardMode> {
        None
    }
}
