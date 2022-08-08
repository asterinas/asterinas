use crate::prelude::*;

/// An interupt request (IRQ) line.
pub struct IrqLine {}

impl IrqLine {
    /// Acquire an interrupt request line.
    ///
    /// # Safety
    ///
    /// This function is marked unsafe as manipulating interrupt lines is
    /// considered a dangerous operation.
    pub unsafe fn acquire(irq_num: u32) -> Arc<Self> {
        todo!()
    }

    /// Get the IRQ number.
    pub fn num(&self) -> u32 {
        todo!()
    }

    /// Register a callback that will be invoked when the IRQ is active.
    ///
    /// A handle to the callback is returned. Dropping the handle
    /// automatically unregisters the callback.
    ///
    /// For each IRQ line, multiple callbacks may be registered.
    pub fn on_active<F>(&self, callback: F) -> IrqCallbackHandle
    where
        F: Fn(&Self),
    {
        todo!()
    }
}

/// The handle to a registered callback for a IRQ line.
///
/// When the handle is dropped, the callback will be unregistered automatically.
#[must_use]
pub struct IrqCallbackHandle {}

impl Drop for IrqCallbackHandle {
    fn drop(&mut self) {
        todo!("unregister the callback")
    }
}
