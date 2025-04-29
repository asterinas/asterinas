// SPDX-License-Identifier: MPL-2.0

//! Handle mouse input.
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::{boxed::Box, vec::Vec};
use core::ops::Deref;

use component::{init_component, ComponentInitError};
use ostd::sync::SpinLock;

mod i8042_mouse;
mod event_type_codes;

static MOUSE_CALLBACKS: SpinLock<Vec<Box<MouseCallback>>> = SpinLock::new(Vec::new());

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    log::error!("This is init in kernel/comps/mouse/src/lib.rs");
    i8042_mouse::init();
    Ok(())
}

/// The callback function for mouse.
pub type MouseCallback = dyn Fn() + Send + Sync;

pub fn register_callback(callback: &'static MouseCallback) {
    log::error!("This is register_callback in kernel/comps/mouse/src/lib.rs");
    MOUSE_CALLBACKS
        .disable_irq()
        .lock()
        .push(Box::new(callback));
}
