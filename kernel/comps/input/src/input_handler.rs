// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::{any::Any, fmt::Debug};

use crate::{InputDevice, input_dev::InputEvent, unregister_handler_class};

/// Errors that can occur when connecting to an input device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectError {
    /// Device is not compatible with this handler class.
    IncompatibleDevice,
    /// Failed to create device node.
    DeviceNodeCreationFailed,
    /// Device is already connected.
    AlreadyConnected,
    /// Other internal error.
    InternalError,
}

/// A trait that represents an input handler class.
///
/// Once registered to the input core (via [`register_handler_class`]), the
/// input handler class will try to connect to each input device (via
/// [`connect`]). If it succeeds, an [`InputHandler`] will be created to handle
/// the input events from that device.
///
/// [`register_handler_class`]: crate::register_handler_class
/// [`connect`]: Self::connect
pub trait InputHandlerClass: Send + Sync + Any + Debug {
    /// Returns the class name of the handler class.
    fn name(&self) -> &str;

    /// Tries to connect to the input device.
    ///
    /// On success, this method will return `Ok()` with a new input handler.
    /// Otherwise, it will return `Err(ConnectError)`.
    fn connect(&self, dev: Arc<dyn InputDevice>) -> Result<Arc<dyn InputHandler>, ConnectError>;

    /// Disconnects from a device.
    fn disconnect(&self, dev: &Arc<dyn InputDevice>);
}

/// A trait that represents an individual input handler instance for a specific device.
///
/// Each handler instance is created by an [`InputHandlerClass`] when it successfully
/// connects to an input device (via [`InputHandlerClass::connect`]). The handler
/// is responsible for processing input events from the associated device.
///
/// [`InputHandlerClass`]: crate::InputHandlerClass
/// [`InputHandlerClass::connect`]: crate::InputHandlerClass::connect
pub trait InputHandler: Send + Sync + Debug {
    /// Handles multiple input events from the device.
    fn handle_events(&self, events: &[InputEvent]);
}

/// An input handler bound with the class that created it.
#[derive(Debug, Clone)]
pub(crate) struct BoundInputHandler {
    pub(crate) handler: Arc<dyn InputHandler>,
    pub(crate) handler_class: Arc<dyn InputHandlerClass>,
}

/// Registered input handler class that can create handlers.
#[derive(Debug)]
pub struct RegisteredInputHandlerClass(pub(crate) Arc<dyn InputHandlerClass>);

impl Drop for RegisteredInputHandlerClass {
    fn drop(&mut self) {
        unregister_handler_class(&self.0).unwrap();
    }
}
