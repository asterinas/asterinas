// SPDX-License-Identifier: MPL-2.0

//! An opt-in input handler that logs events for debugging.

use alloc::sync::Arc;

use crate::{
    input_dev::{InputDevice, InputEvent},
    input_handler::{ConnectError, InputHandler, InputHandlerClass, RegisteredInputHandlerClass},
};

/// Registers an event logger for existing and subsequently added input devices.
///
/// Dropping the returned registration detaches the logger from all devices.
/// Event logging includes keyboard input, so enable it only for debugging.
pub fn register() -> RegisteredInputHandlerClass {
    crate::register_handler_class(Arc::new(EvbugClass))
}

#[derive(Debug)]
struct EvbugClass;

impl InputHandlerClass for EvbugClass {
    fn name(&self) -> &str {
        "evbug"
    }

    fn connect(&self, dev: Arc<dyn InputDevice>) -> Result<Arc<dyn InputHandler>, ConnectError> {
        let handler = Arc::try_new(EvbugHandler {
            device: dev.clone(),
        })
        .map_err(|_| ConnectError::InternalError)?;

        let name = device_name(dev.as_ref());
        let phys = nonempty_or_unknown(dev.phys());
        ostd::debug!("evbug: Connected device: {name} ({name} at {phys})");
        Ok(handler)
    }

    fn disconnect(&self, dev: &Arc<dyn InputDevice>) {
        ostd::debug!("evbug: Disconnected device: {}", device_name(dev.as_ref()));
    }
}

#[derive(Debug)]
struct EvbugHandler {
    device: Arc<dyn InputDevice>,
}

impl InputHandler for EvbugHandler {
    fn handle_events(&self, events: &[InputEvent]) {
        let name = device_name(self.device.as_ref());
        for event in events {
            let (event_type, code, value) = event.to_raw();
            ostd::debug!(
                "evbug: Event. Dev: {name}, Type: {event_type}, Code: {code}, Value: {value}"
            );
        }
    }
}

fn device_name(device: &dyn InputDevice) -> &str {
    nonempty_or_unknown(device.name())
}

fn nonempty_or_unknown(value: &str) -> &str {
    if value.is_empty() { "unknown" } else { value }
}
