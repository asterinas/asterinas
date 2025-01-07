// SPDX-License-Identifier: MPL-2.0

//! The framebuffer console of Asterinas.
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::vec::Vec;
use core::ops::Deref;

use aster_console::{AnyConsoleDevice, ConsoleCallback};
use aster_keyboard::InputKey;
use component::{init_component, ComponentInitError};
use ostd::{mm::VmReader, sync::SpinLock};
use spin::Once;

pub static CONSOLE_NAME: &str = "Framebuffer-Console";

static CONSOLE_CALLBACKS: Once<SpinLock<Vec<&'static ConsoleCallback>>> = Once::new();

#[init_component]
fn framebuffer_init() -> Result<(), ComponentInitError> {
    CONSOLE_CALLBACKS.call_once(|| SpinLock::new(Vec::new()));
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
        let Some(callbacks) = CONSOLE_CALLBACKS.get() else {
            return;
        };

        callbacks.disable_irq().lock().push(callback);
    }
}

fn handle_keyboard_input(key: InputKey) {
    if key == InputKey::Nul {
        return;
    }

    let Some(callbacks) = CONSOLE_CALLBACKS.get() else {
        return;
    };

    let buffer = key.deref();
    for callback in callbacks.disable_irq().lock().iter() {
        let reader = VmReader::from(buffer);
        callback(reader);
    }
}
