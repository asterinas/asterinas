// SPDX-License-Identifier: MPL-2.0

//! Handle keyboard input.
#![no_std]
#![deny(unsafe_code)]
#![cfg(target_arch = "x86_64")]

extern crate alloc;

use component::{ComponentInitError, init_component};

mod controller;
mod keyboard;
mod mouse;
mod ps2;

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    if let Err(err) = controller::init() {
        log::warn!("i8042 controller initialization failed: {:?}", err);
    }
    Ok(())
}
