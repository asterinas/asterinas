// SPDX-License-Identifier: MPL-2.0

//! Guest timers.

use crate::arch::vm::GuestTimerInstant;

/// A guest timer for [`GuestMode`](crate::arch::vm::GuestMode).
///
/// Methods can run with preemption and local IRQs disabled and must not block.
pub trait GuestTimer {
    /// Returns a deadline in the guest's TSC timeline, or `None` for no deadline.
    fn poll_deadline(&self, current: GuestTimerInstant) -> Option<GuestTimerInstant>;
}

/// A guest timer with no pending deadline.
pub struct DummyGuestTimer;

impl GuestTimer for DummyGuestTimer {
    fn poll_deadline(&self, _current: GuestTimerInstant) -> Option<GuestTimerInstant> {
        None
    }
}
