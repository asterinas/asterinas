// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec::Vec};
use core::fmt::Debug;

use ostd::sync::RwLock;

use crate::{input_dev::RegisteredInputDevice, InputDevice, InputHandler, InputHandlerClass};

/// Registry entry for each registered device
///
/// This serves as the connection point between devices and their handlers.
#[derive(Debug)]
struct InputDeviceRegistry {
    /// The input device
    device: Arc<dyn InputDevice>,
    /// Handlers connected to this device
    handlers: Arc<RwLock<Vec<Arc<dyn InputHandler>>>>,
}

#[derive(Debug)]
pub struct InputCore {
    /// All registered devices with their handlers
    devices: RwLock<Vec<InputDeviceRegistry>>,
    /// All registered handler classes
    handler_classes: RwLock<Vec<Arc<dyn InputHandlerClass>>>,
}

impl InputCore {
    /// Create a new input core
    pub(crate) fn new() -> Self {
        Self {
            devices: RwLock::new(Vec::new()),
            handler_classes: RwLock::new(Vec::new()),
        }
    }

    /// Register a new handler class
    pub(crate) fn register_handler_class(&self, handler_class: Arc<dyn InputHandlerClass>) {
        self.handler_classes.write().push(handler_class.clone());

        // Connect to all existing devices
        let devices = self.devices.read();
        for device_registry in devices.iter() {
            if let Ok(handler) = handler_class.connect(device_registry.device.clone()) {
                device_registry.handlers.write().push(handler);
            }
        }

        log::info!("Registered handler class: {}", handler_class.name());
    }

    /// Unregister a handler class
    pub(crate) fn unregister_handler_class(&self, handler_class: &Arc<dyn InputHandlerClass>) {
        let class_name = handler_class.name();

        // Remove from handler classes
        self.handler_classes
            .write()
            .retain(|h| h.name() != class_name);

        // Disconnect from all devices and remove handlers
        let devices = self.devices.read();
        for device_registry in devices.iter() {
            // Notify handler class about disconnection
            handler_class.disconnect(&device_registry.device);

            // Remove handlers belonging to this class
            device_registry
                .handlers
                .write()
                .retain(|h| h.class_name() != class_name);
        }

        log::info!("Unregistered handler class: {}", handler_class.name());
    }

    /// Register a new input device
    pub(crate) fn register_device(&self, device: Arc<dyn InputDevice>) -> RegisteredInputDevice {
        // Connect all existing handler classes
        let handler_classes = self.handler_classes.read();
        let mut connected_handlers = Vec::new();

        for handler_class in handler_classes.iter() {
            if let Ok(handler) = handler_class.connect(device.clone()) {
                connected_handlers.push(handler);
            }
        }

        let handlers = Arc::new(RwLock::new(connected_handlers));

        let new_registry = InputDeviceRegistry {
            device: device.clone(),
            handlers: handlers.clone(),
        };

        // Add to devices registry
        self.devices.write().push(new_registry);

        RegisteredInputDevice::new(device, handlers)
    }

    /// Unregister an input device
    pub(crate) fn unregister_device(&self, device: &Arc<dyn InputDevice>) {
        let mut devices = self.devices.write();

        // Find the device to remove
        if let Some(pos) = devices
            .iter()
            .position(|registry| Arc::ptr_eq(&registry.device, device))
        {
            let device_registry = devices.remove(pos);
            device_registry.handlers.write().clear();

            // Disconnect all handler classes from this device
            let handler_classes = self.handler_classes.read();
            for handler_class in handler_classes.iter() {
                handler_class.disconnect(&device_registry.device);
            }

            log::info!("Unregistered input device: {}", device.name());
        } else {
            log::warn!(
                "Device {} not found in registry, cannot unregister",
                device.name()
            );
        }
    }

    /// Get device count
    pub(crate) fn device_count(&self) -> usize {
        self.devices.read().len()
    }

    /// Get handler class count
    pub(crate) fn handler_class_count(&self) -> usize {
        self.handler_classes.read().len()
    }

    /// Get all registered devices
    pub(crate) fn all_devices(&self) -> Vec<Arc<dyn InputDevice>> {
        let devices = self.devices.read();
        devices
            .iter()
            .map(|registry| registry.device.clone())
            .collect()
    }
}
