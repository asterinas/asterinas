// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, vec::Vec};
use core::fmt::Debug;

use ostd::sync::{RwLock, SpinLock, WriteIrqDisabled};

use crate::{
    input_dev::RegisteredInputDevice,
    input_handler::{InputHandler, InputHandlerClass},
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
    handlers: Arc<RwLock<Vec<Arc<dyn InputHandler>>, WriteIrqDisabled>>,
}

/// The core component of the input subsystem.
///
/// `InputCore` manages all registered input devices and handler classes.
/// It coordinates the connection between devices and handlers, and routes
/// input events from devices to their associated handlers.
#[derive(Debug)]
pub(crate) struct InputCore {
    /// Serialize register and unregister operations to avoid race.
    ops_lock: SpinLock<()>,
    /// All registered devices with their associated handlers.
    devices: RwLock<Vec<InputDeviceRegistry>>,
    /// All registered handler classes that can connect to devices.
    handler_classes: RwLock<Vec<Arc<dyn InputHandlerClass>>>,
}

impl InputCore {
    /// Creates a new input core.
    pub(crate) fn new() -> Self {
        Self {
            ops_lock: SpinLock::new(()),
            devices: RwLock::new(Vec::new()),
            handler_classes: RwLock::new(Vec::new()),
        }
    }

    /// Registers a new handler class.
    pub(crate) fn register_handler_class(&self, handler_class: Arc<dyn InputHandlerClass>) {
        let _ops_guard = self.ops_lock.lock();
        self.handler_classes.write().push(handler_class.clone());

        // Connect to all existing devices
        let devices = self.devices.read();
        for device_registry in devices.iter() {
            match handler_class.connect(device_registry.device.clone()) {
                Ok(handler) => {
                    device_registry.handlers.write().push(handler);
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
    }

    /// Unregisters a handler class.
    pub(crate) fn unregister_handler_class(
        &self,
        handler_class: &Arc<dyn InputHandlerClass>,
    ) -> Option<Arc<dyn InputHandlerClass>> {
        let _ops_guard = self.ops_lock.lock();
        let class_name = handler_class.name();

        // Remove from handler classes.
        let mut classes = self.handler_classes.write();
        if let Some(pos) = classes.iter().position(|h| Arc::ptr_eq(h, handler_class)) {
            let removed = classes.remove(pos);

            // Disconnect from all devices and removes handlers.
            let devices = self.devices.read();
            for device_registry in devices.iter() {
                // Notify handler class about disconnection.
                handler_class.disconnect(&device_registry.device);

                // Remove handlers belonging to this class.
                device_registry
                    .handlers
                    .write()
                    .retain(|h| h.class_name() != class_name);
            }

            log::info!("Input: unregistered handler class {}", handler_class.name());
            return Some(removed);
        }

        None
    }

    /// Registers a new input device.
    pub(crate) fn register_device(&self, device: Arc<dyn InputDevice>) -> RegisteredInputDevice {
        let _ops_guard = self.ops_lock.lock();
        // Connects all existing handler classes.
        let handler_classes = self.handler_classes.read();
        let mut connected_handlers = Vec::new();

        log::info!("Input: registered device {}", device.name());

        for handler_class in handler_classes.iter() {
            match handler_class.connect(device.clone()) {
                Ok(handler) => {
                    connected_handlers.push(handler);
                    log::info!(
                        "Input: successfully connected handler class {} to device {}",
                        handler_class.name(),
                        device.name()
                    );
                }
                Err(e) => {
                    log::warn!(
                        "Input: failed to connect handler class {} to device {}: {:?}",
                        handler_class.name(),
                        device.name(),
                        e
                    );
                }
            }
        }

        let handlers = Arc::new(RwLock::<_, WriteIrqDisabled>::new(connected_handlers));

        let new_registry = InputDeviceRegistry {
            device: device.clone(),
            handlers: handlers.clone(),
        };

        // Add to devices registry.
        self.devices.write().push(new_registry);

        RegisteredInputDevice::new(device, handlers)
    }

    /// Unregisters an input device.
    pub(crate) fn unregister_device(
        &self,
        device: &Arc<dyn InputDevice>,
    ) -> Option<Arc<dyn InputDevice>> {
        let _ops_guard = self.ops_lock.lock();
        let mut devices = self.devices.write();

        // Find the device to remove.
        if let Some(pos) = devices
            .iter()
            .position(|registry| Arc::ptr_eq(&registry.device, device))
        {
            let device_registry = devices.remove(pos);
            device_registry.handlers.write().clear();

            // Disconnect all handler classes from this device.
            let handler_classes = self.handler_classes.read();
            for handler_class in handler_classes.iter() {
                handler_class.disconnect(&device_registry.device);
            }

            log::info!("Input: unregistered device {}", device.name());
            Some(device_registry.device)
        } else {
            None
        }
    }

    /// Gets device count.
    pub(crate) fn count_devices(&self) -> usize {
        self.devices.read().len()
    }

    /// Gets handler class count.
    pub(crate) fn count_handler_class(&self) -> usize {
        self.handler_classes.read().len()
    }

    /// Gets all registered devices.
    pub(crate) fn all_devices(&self) -> Vec<Arc<dyn InputDevice>> {
        let devices = self.devices.read();
        devices
            .iter()
            .map(|registry| registry.device.clone())
            .collect()
    }
}
