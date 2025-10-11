// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec::Vec};
use core::fmt::Debug;

use ostd::sync::{RwLock, WriteIrqDisabled};

use crate::{
    input_dev::RegisteredInputDevice,
    input_handler::{BoundInputHandler, InputHandlerClass},
    InputDevice,
};

/// Registry entry for each registered device.
///
/// This serves as the connection point between devices and their handlers.
#[derive(Debug)]
struct InputDeviceRegistry {
    /// The input device.
    device: Arc<dyn InputDevice>,
    /// Handlers connected to this device.
    handlers: Arc<RwLock<Vec<BoundInputHandler>, WriteIrqDisabled>>,
}

/// The core component of the input subsystem.
///
/// `InputCore` manages all registered input devices and handler classes.
/// It coordinates the connection between devices and handlers, and routes
/// input events from devices to their associated handlers.
#[derive(Debug)]
pub(crate) struct InputCore {
    /// All registered devices with their associated handlers.
    devices: Vec<InputDeviceRegistry>,
    /// All registered handler classes that can connect to devices.
    handler_classes: Vec<Arc<dyn InputHandlerClass>>,
}

impl InputCore {
    /// Creates a new input core.
    pub(crate) fn new() -> Self {
        Self {
            devices: Vec::new(),
            handler_classes: Vec::new(),
        }
    }

    /// Registers a new handler class.
    pub(crate) fn register_handler_class(&mut self, handler_class: Arc<dyn InputHandlerClass>) {
        // Connect to all existing devices
        for device_registry in self.devices.iter() {
            match handler_class.connect(device_registry.device.clone()) {
                Ok(handler) => {
                    device_registry.handlers.write().push(BoundInputHandler {
                        handler,
                        handler_class: handler_class.clone(),
                    });
                    log::info!(
                        "Input: successfully connected handler class {} to device {}",
                        handler_class.name(),
                        device_registry.device.name()
                    );
                }
                Err(e) => {
                    log::info!(
                        "Input: failed to connect handler class {} to device {}: {:?}",
                        handler_class.name(),
                        device_registry.device.name(),
                        e
                    );
                }
            }
        }

        log::info!("Input: registered handler class {}", handler_class.name());
        self.handler_classes.push(handler_class);
    }

    /// Unregisters a handler class.
    pub(crate) fn unregister_handler_class(
        &mut self,
        handler_class: &Arc<dyn InputHandlerClass>,
    ) -> Option<Arc<dyn InputHandlerClass>> {
        // Find the handler class and remove it.
        let pos = self
            .handler_classes
            .iter()
            .position(|h| Arc::ptr_eq(h, handler_class))?;
        let handler_class = self.handler_classes.swap_remove(pos);

        for device_registry in self.devices.iter() {
            let mut handlers = device_registry.handlers.write();
            let Some(pos) = handlers
                .iter()
                .position(|h| Arc::ptr_eq(&h.handler_class, &handler_class))
            else {
                continue;
            };
            let handler = handlers.swap_remove(pos);
            drop(handlers);
            drop(handler);

            handler_class.disconnect(&device_registry.device);
        }

        log::info!("Input: unregistered handler class {}", handler_class.name());
        Some(handler_class)
    }

    /// Registers a new input device.
    pub(crate) fn register_device(
        &mut self,
        device: Arc<dyn InputDevice>,
    ) -> RegisteredInputDevice {
        // Connect all existing handler classes.
        let mut connected_handlers = Vec::new();
        for handler_class in self.handler_classes.iter() {
            match handler_class.connect(device.clone()) {
                Ok(handler) => {
                    connected_handlers.push(BoundInputHandler {
                        handler,
                        handler_class: handler_class.clone(),
                    });
                    log::info!(
                        "Input: successfully connected handler class {} to device {}",
                        handler_class.name(),
                        device.name()
                    );
                }
                Err(e) => {
                    log::info!(
                        "Input: failed to connect handler class {} to device {}: {:?}",
                        handler_class.name(),
                        device.name(),
                        e
                    );
                }
            }
        }
        let handlers = Arc::new(RwLock::new(connected_handlers));

        // Add the device registry.
        let new_registry = InputDeviceRegistry {
            device: device.clone(),
            handlers: handlers.clone(),
        };
        self.devices.push(new_registry);

        log::info!("Input: registered device {}", device.name());
        RegisteredInputDevice::new(device, handlers)
    }

    /// Unregisters an input device.
    pub(crate) fn unregister_device(
        &mut self,
        device: &Arc<dyn InputDevice>,
    ) -> Option<Arc<dyn InputDevice>> {
        // Find the device and remove it.
        let pos = self
            .devices
            .iter()
            .position(|registry| Arc::ptr_eq(&registry.device, device))?;
        let device_registry = self.devices.swap_remove(pos);

        // Take all handlers connected to this device and clear the list.
        let mut handlers = device_registry.handlers.write();
        let bound_handlers = core::mem::take(&mut *handlers);
        drop(handlers);

        // Disconnect handler classes that were connected.
        for bound_handler in bound_handlers.into_iter() {
            bound_handler
                .handler_class
                .disconnect(&device_registry.device);
        }

        log::info!("Input: unregistered device {}", device.name());
        Some(device_registry.device)
    }

    /// Counts the number of registered devices.
    pub(crate) fn count_devices(&self) -> usize {
        self.devices.len()
    }

    /// Counts the number of registered handler classes.
    pub(crate) fn count_handler_class(&self) -> usize {
        self.handler_classes.len()
    }

    /// Gets all registered devices.
    pub(crate) fn all_devices(&self) -> Vec<Arc<dyn InputDevice>> {
        self.devices
            .iter()
            .map(|registry| registry.device.clone())
            .collect()
    }
}
