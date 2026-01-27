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
//! let key_event = InputEvent::from_key_and_status(linux_key, key_status);
//! registered_device.submit_events(&[key_event]);
//! ```
//!
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;
#[macro_use]
extern crate ostd_pod;

pub mod event_type_codes;
mod input_core;
pub mod input_dev;
pub mod input_handler;

use alloc::{sync::Arc, vec::Vec};

use component::{ComponentInitError, init_component};
use ostd::sync::Mutex;
use spin::Once;

use self::input_core::InputCore;
use crate::{
    input_dev::{InputDevice, RegisteredInputDevice},
    input_handler::{InputHandlerClass, RegisteredInputHandlerClass},
};

/// Registers a handler class.
pub fn register_handler_class(
    handler_class: Arc<dyn InputHandlerClass>,
) -> RegisteredInputHandlerClass {
    let component = COMPONENT.get().unwrap();
    component
        .input_core
        .lock()
        .register_handler_class(handler_class.clone());
    RegisteredInputHandlerClass(handler_class)
}

/// Unregisters a handler class.
pub(crate) fn unregister_handler_class(
    handler_class: &Arc<dyn InputHandlerClass>,
) -> Option<Arc<dyn InputHandlerClass>> {
    let component = COMPONENT.get().unwrap();
    component
        .input_core
        .lock()
        .unregister_handler_class(handler_class)
}

/// Registers an input device.
pub fn register_device(device: Arc<dyn InputDevice>) -> RegisteredInputDevice {
    let component = COMPONENT.get().unwrap();
    component.input_core.lock().register_device(device)
}

/// Unregisters an input device.
pub(crate) fn unregister_device(device: &Arc<dyn InputDevice>) -> Option<Arc<dyn InputDevice>> {
    let component = COMPONENT.get().unwrap();
    component.input_core.lock().unregister_device(device)
}

/// Counts the number of registered devices.
pub fn count_devices() -> usize {
    let component = COMPONENT.get().unwrap();
    component.input_core.lock().count_devices()
}

/// Counts the number of registered handler classes.
pub fn count_handler_class() -> usize {
    let component = COMPONENT.get().unwrap();
    component.input_core.lock().count_handler_class()
}

/// Gets all registered devices.
pub fn all_devices() -> Vec<Arc<dyn InputDevice>> {
    let component = COMPONENT.get().unwrap();
    component.input_core.lock().all_devices()
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
    input_core: Mutex<InputCore>,
}

impl Component {
    pub fn init() -> Result<Self, ComponentInitError> {
        Ok(Self {
            input_core: Mutex::new(InputCore::new()),
        })
    }
}
