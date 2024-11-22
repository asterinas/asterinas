// SPDX-License-Identifier: MPL-2.0

//! The log service for Asterinas.
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use component::{init_component, ComponentInitError};

mod console;
mod log_service;

pub use console::_print;

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    log_service::init();
    Ok(())
}
