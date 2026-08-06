// SPDX-License-Identifier: MPL-2.0

//! Handle keyboard input.
#![no_std]
#![deny(unsafe_code)]
#![cfg(target_arch = "x86_64")]

extern crate alloc;

use component::{ComponentInitError, init_component};
use ostd::power;

use self::controller::I8042_CONTROLLER;

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "i8042: "
    };
}

mod controller;
mod keyboard;
mod mouse;
mod ps2;

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    if let Err(err) = controller::init() {
        ostd::warn!("i8042 controller initialization failed: {:?}", err);
    }
    Ok(())
}

/// Attempts to reset the CPU via the i8042 PS/2 controller.
pub fn try_cpu_reset(_code: power::ExitCode) {
    if let Some(controller) = I8042_CONTROLLER.get() {
        controller.lock().reset_cpu();
    }
}
