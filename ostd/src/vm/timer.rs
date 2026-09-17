// SPDX-License-Identifier: MPL-2.0

use crate::arch::vm::GuestTimerInstant;

/// The guest timer policy supplied to [`super::GuestMode`].
///
/// Methods can run with preemption and local IRQs disabled and must not block.
pub trait GuestTimerPort {
    /// Returns a deadline in the guest's TSC timeline, or `None` for no deadline.
    fn poll_deadline(&self, _current: GuestTimerInstant) -> Option<GuestTimerInstant> {
        None
    }
}
