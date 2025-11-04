// SPDX-License-Identifier: MPL-2.0

//! Handle keyboard input.
#![no_std]
#![deny(unsafe_code)]
#![cfg(target_arch = "x86_64")]

extern crate alloc;

use component::{init_component, ComponentInitError};

mod controller;
mod keyboard;
mod mouse;

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    if let Err(err) = controller::init() {
        log::warn!("i8042 controller initialization failed: {:?}", err);
    }
    Ok(())
}
