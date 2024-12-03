// SPDX-License-Identifier: MPL-2.0

use crate::{
    task::{disable_preempt, DisabledPreemptGuard},
    trap::{disable_local, DisabledLocalIrqGuard},
};

/// A guardian that denotes the guard behavior for holding the spin lock.
pub trait Guardian {
    /// The guard type.
    type Guard: GuardTransfer;

    /// Creates a new guard.
    fn guard() -> Self::Guard;
}

/// The Guard can be transferred atomically.
pub trait GuardTransfer {
    /// Atomically transfers the current guard to a new instance.
    ///
    /// This function ensures that there are no 'gaps' between the destruction of the old guard and
    /// the creation of the new guard, thereby maintaining the atomicity of guard transitions.
    ///
    /// The original guard must be dropped immediately after calling this method.
    fn transfer_to(&mut self) -> Self;
}

/// A guardian that disables preemption while holding the spin lock.
pub struct PreemptDisabled;

impl Guardian for PreemptDisabled {
    type Guard = DisabledPreemptGuard;

    fn guard() -> Self::Guard {
        disable_preempt()
    }
}

/// A guardian that disables IRQs while holding the spin lock.
///
/// This guardian would incur a certain time overhead over
/// [`PreemptDisabled']. So prefer avoiding using this guardian when
/// IRQ handlers are allowed to get executed while holding the
/// lock. For example, if a lock is never used in the interrupt
/// context, then it is ok not to use this guardian in the process context.
pub struct LocalIrqDisabled;

impl Guardian for LocalIrqDisabled {
    type Guard = DisabledLocalIrqGuard;

    fn guard() -> Self::Guard {
        disable_local()
    }
}
