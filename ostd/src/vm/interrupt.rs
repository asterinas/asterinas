// SPDX-License-Identifier: MPL-2.0

//! Guest interrupt sources.

use crate::arch::vm::GuestInterrupt;

/// A source of guest interrupts for [`GuestMode`](crate::arch::vm::GuestMode).
///
/// Methods can run with preemption and local IRQs disabled and must not block.
pub trait GuestInterruptSource {
    /// Returns a pending external interrupt without consuming it.
    fn query_pending_interrupt(&self) -> Option<GuestInterrupt>;

    /// Marks an interrupt as accepted after successful guest entry.
    fn accept_interrupt(&self, interrupt: GuestInterrupt);
}

/// A guest interrupt source with no pending interrupts.
pub struct DummyGuestInterruptSource;

impl GuestInterruptSource for DummyGuestInterruptSource {
    fn query_pending_interrupt(&self) -> Option<GuestInterrupt> {
        None
    }

    fn accept_interrupt(&self, _interrupt: GuestInterrupt) {}
}
