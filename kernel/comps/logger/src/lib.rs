// SPDX-License-Identifier: MPL-2.0

//! The logger implementation for Asterinas.
//!
//! This logger now has the most basic logging functionality, controls the output
//! based on the globally set log level. Different log levels will be represented
//! with different colors if enabling `log_color` feature.
//!
//! This logger guarantees _atomicity_ under concurrency: messages are always
//! printed in their entirety without being mixed with messages generated
//! concurrently on other cores.
//!
//! IRQs are disabled while printing. So do not print long log messages.
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use component::{init_component, ComponentInitError};

mod aster_logger;
mod console;

pub use console::_print;

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    aster_logger::init();
    Ok(())
}
