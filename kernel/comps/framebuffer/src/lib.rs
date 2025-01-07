// SPDX-License-Identifier: MPL-2.0

//! The framebuffer console of Asterinas.
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::vec::Vec;

use aster_console::{AnyConsoleDevice, ConsoleCallback};
use component::{init_component, ComponentInitError};
use ostd::sync::SpinLock;
use spin::Once;

pub static CONSOLE_NAME: &str = "Framebuffer-Console";

static CONSOLE_CALLBACKS: Once<SpinLock<Vec<&'static ConsoleCallback>>> = Once::new();

#[init_component]
fn framebuffer_init() -> Result<(), ComponentInitError> {
    CONSOLE_CALLBACKS.call_once(|| SpinLock::new(Vec::new()));
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
