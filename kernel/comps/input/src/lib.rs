// SPDX-License-Identifier: MPL-2.0

//! The input devices of Asterinas.
//!
//! This crate provides a comprehensive input subsystem for handling various input devices,
//! including keyboards, mice, etc. It implements an event-driven architecture similar to
//! the Linux input subsystem.
//!
//! # Architecture
//!
//! ```text
//! Input Device → Input Core → Input Handler
//!      ↓             ↓            ↓
//!   Hardware    Event Router   Event Consumer
//!                              (e.g., evdev)
//! ```
//!
//! # Example Usage
//!
//! ```
//! // Register an input device
//! let device = Arc::new(MyInputDevice::new());
//! let registered_device = input::register_device(device);
//!
//! // Register an input handler
//! let handler = Arc::new(MyInputHandler::new());
//! input::register_handler(handler);
//!
//! // Submit a key event from device
//! let key_event = InputEvent::key(linux_key, key_status);
//! registered_device.submit_event(&key_event);
//! ```
//!
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

pub mod event_type_codes;
mod input_core;
pub mod input_dev;
pub mod input_handler;

use alloc::{sync::Arc, vec::Vec};

use component::{init_component, ComponentInitError};
pub use event_type_codes::*;
pub use input_dev::{InputCapability, InputDevice, InputEvent, InputId, RegisteredInputDevice};
pub use input_handler::{InputHandler, InputHandlerClass};
use spin::Once;

use self::input_core::InputCore;

/// Register a handler class.
pub fn register_handler_class(handler_class: Arc<dyn InputHandlerClass>) {
    COMPONENT
        .get()
        .unwrap()
        .input_core
        .register_handler_class(handler_class)
}

/// Unregister a handler class.
pub fn unregister_handler_class(handler_class: &Arc<dyn InputHandlerClass>) {
    COMPONENT
        .get()
        .unwrap()
        .input_core
        .unregister_handler_class(handler_class)
}

/// Register an input device.
pub fn register_device(device: Arc<dyn InputDevice>) -> RegisteredInputDevice {
    COMPONENT.get().unwrap().input_core.register_device(device)
}

/// Unregister an input device.
pub fn unregister_device(device: &Arc<dyn InputDevice>) {
    COMPONENT
        .get()
        .unwrap()
        .input_core
        .unregister_device(device)
}

/// Get device count.
pub fn device_count() -> usize {
    COMPONENT.get().unwrap().input_core.device_count()
}

/// Get handler class count.
pub fn handler_class_count() -> usize {
    COMPONENT.get().unwrap().input_core.handler_class_count()
}

/// Get all registered devices.
pub fn all_devices() -> Vec<Arc<dyn InputDevice>> {
    COMPONENT.get().unwrap().input_core.all_devices()
}

static COMPONENT: Once<Component> = Once::new();

#[init_component]
fn component_init() -> Result<(), ComponentInitError> {
    let component = Component::init()?;
    COMPONENT.call_once(|| component);
    Ok(())
}

#[derive(Debug)]
struct Component {
    input_core: InputCore,
}

impl Component {
    pub fn init() -> Result<Self, ComponentInitError> {
        Ok(Self {
            input_core: InputCore::new(),
        })
    }
}
