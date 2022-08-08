//! Timer.

use crate::prelude::*;
use core::time::Duration;

/// A timer invokes a callback function after a specified span of time elapsed.
///
/// A new timer is initially inactive. Only after a timeout value is set with
/// the `set` method can the timer become active and the callback function
/// be triggered.
///
/// Timers are one-shot. If the time is out, one has to set the timer again
/// in order to trigger the callback again.
pub struct Timer {}

impl Timer {
    /// Creates a new instance, given a callback function.
    pub fn new<F>(f: F) -> Result<Self>
    where
        F: FnMut(&Self),
    {
        todo!()
    }

    /// Set a timeout value.
    ///
    /// If a timeout value is already set, the timeout value will be refreshed.
    pub fn set(&self, timeout: Duration) {
        todo!()
    }

    /// Returns the remaining timeout value.
    ///
    /// If the timer is not set, then the remaining timeout value is zero.
    pub fn remain(&self) -> Duration {
        todo!()
    }

    /// Clear the timeout value.
    pub fn clear(&self) {
        todo!()
    }
}
