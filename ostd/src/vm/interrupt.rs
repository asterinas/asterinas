// SPDX-License-Identifier: MPL-2.0

use crate::arch::vm::GuestInterrupt;

/// The guest interrupt policy supplied to [`super::GuestMode`].
///
/// Methods can run with preemption and local IRQs disabled and must not block.
pub trait GuestInterruptPort {
    /// Returns a pending external interrupt without consuming it.
    fn query_pending_interrupt(&self) -> Option<GuestInterrupt> {
        None
    }

    /// Marks an interrupt as accepted after successful guest entry.
    fn accept_interrupt(&self, _interrupt: GuestInterrupt) {}
}
