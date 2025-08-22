// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::{any::Any, fmt::Debug};

use crate::{InputDevice, InputEvent};

/// Handler class trait - represents a type of input handler (e.g., evdev, kbd)
/// Each handler class can create multiple handler instances, one per device.
pub trait InputHandlerClass: Send + Sync + Any + Debug {
    /// Handler class name (e.g., "evdev", "kbd")
    fn name(&self) -> &str;

    /// Try to connect to a device and create a handler instance
    /// Returns Ok(handler) if compatible, Err() if incompatible
    #[expect(clippy::result_unit_err)]
    fn connect(&self, dev: Arc<dyn InputDevice>) -> Result<Arc<dyn InputHandler>, ()>;

    /// Disconnect from a device
    fn disconnect(&self, dev: &Arc<dyn InputDevice>);
}

/// Individual handler instance for a specific device
pub trait InputHandler: Send + Sync + Debug {
    /// Get the handler class name this handler belongs to
    fn class_name(&self) -> &str;

    /// Handle single event from the device
    fn handle_event(&self, event: &InputEvent);

    /// Handle multiple events from the device
    fn handle_events(&self, events: &[InputEvent]);
}
