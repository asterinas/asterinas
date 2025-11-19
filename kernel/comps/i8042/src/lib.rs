// SPDX-License-Identifier: MPL-2.0

//! Handle keyboard input.
#![no_std]
#![deny(unsafe_code)]
#![allow(incomplete_features)]
#![feature(array_try_from_fn, generic_const_exprs)]

extern crate alloc;

use component::{init_component, ComponentInitError};
#[cfg(target_arch = "x86_64")]
mod controller;
#[cfg(target_arch = "x86_64")]
mod keyboard;
#[cfg(target_arch = "x86_64")]
mod mouse;

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    #[cfg(target_arch = "x86_64")]
    {
        if let Err(err) = controller::init() {
            log::warn!("i8042 controller initialization failed: {:?}", err);
        }
    }
    Ok(())
}
