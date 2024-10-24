// SPDX-License-Identifier: MPL-2.0

//! Logger System
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use component::{init_component, ComponentInitError};

mod console;
mod filter;
mod filter_logger;

pub use console::_print;

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    filter_logger::init();
    Ok(())
}
