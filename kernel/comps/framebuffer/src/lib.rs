// SPDX-License-Identifier: MPL-2.0

//! The framebuffer console of Asterinas.
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::vec::Vec;

use aster_console::{AnyConsoleDevice, ConsoleCallback};
use aster_keyboard::Key;
use component::{init_component, ComponentInitError};
use ostd::{mm::VmReader, sync::SpinLock};
use spin::Once;

pub static CONSOLE_NAME: &str = "Framebuffer-Console";

static FRAMEBUFFER_CONSOLE_CALLBACKS: Once<SpinLock<Vec<&'static ConsoleCallback>>> = Once::new();

#[init_component]
fn framebuffer_console_init() -> Result<(), ComponentInitError> {
    FRAMEBUFFER_CONSOLE_CALLBACKS.call_once(|| SpinLock::new(Vec::new()));
    aster_keyboard::register_callback(&handle_keyboard_input);
    Ok(())
}

#[derive(Debug, Default)]
pub struct FramebufferConsole;

impl FramebufferConsole {
    pub fn new() -> Self {
        Self
    }
}

impl AnyConsoleDevice for FramebufferConsole {
    fn send(&self, buf: &[u8]) {
        // TODO: handle ANSI escape characters
        for &ch in buf.iter() {
            if ch != 0 {
                let char = char::from_u32(ch as u32).unwrap();
                ostd::arch::framebuffer::print(format_args!("{}", char));
            }
        }
    }

    fn register_callback(&self, callback: &'static ConsoleCallback) {
        let Some(callbacks) = FRAMEBUFFER_CONSOLE_CALLBACKS.get() else {
            return;
        };

        callbacks.disable_irq().lock().push(callback);
    }
}

fn handle_keyboard_input(key: Key) {
    let Some(callbacks) = FRAMEBUFFER_CONSOLE_CALLBACKS.get() else {
        return;
    };

    let mut char = [0u8];
    let buffer = match key {
        Key::Char(ch) | Key::Ctrl(ch) => {
            char[0] = ch as u8;
            char.as_slice()
        }
        Key::Enter => [0xD].as_slice(),
        Key::BackSpace => [0x7F].as_slice(),
        Key::Escape => [0x1B].as_slice(),
        Key::Up => [0x1B, 0x5B, 0x41].as_slice(),
        Key::Down => [0x1B, 0x5B, 0x42].as_slice(),
        Key::Right => [0x1B, 0x5B, 0x43].as_slice(),
        Key::Left => [0x1B, 0x5B, 0x44].as_slice(),
        _ => {
            log::debug!("unsupported keyboard input");
            return;
        }
    };

    for callback in callbacks.disable_irq().lock().iter() {
        let reader = VmReader::from(buffer);
        callback(reader);
    }
}
