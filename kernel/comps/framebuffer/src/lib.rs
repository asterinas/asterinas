// SPDX-License-Identifier: MPL-2.0

//! The framebuffer of Asterinas.
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "framebuffer: "
    };
}

mod ansi_escape;
pub mod console;
pub mod font;
pub mod framebuffer;
pub mod mode;
pub mod pixel;

use component::{ComponentInitError, init_component};

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    framebuffer::init();
    Ok(())
}
