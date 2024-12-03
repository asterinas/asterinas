// SPDX-License-Identifier: MPL-2.0

use crate::{
    task::{disable_preempt, DisabledPreemptGuard},
    trap::{disable_local, DisabledLocalIrqGuard},
};

/// A guardian that denotes the guard behavior for holding a lock.
pub trait Guardian {
    /// The guard type for holding a spin lock or a write lock.
    type Guard: GuardTransfer;
    /// The guard type for holding a read lock.
    type ReadGuard: GuardTransfer;

    /// Creates a new guard.
    fn guard() -> Self::Guard;
    /// Creates a new read guard.
    fn read_guard() -> Self::ReadGuard;
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

/// A guardian that disables preemption while holding a lock.
pub struct PreemptDisabled;

impl Guardian for PreemptDisabled {
    type Guard = DisabledPreemptGuard;
    type ReadGuard = DisabledPreemptGuard;

    fn guard() -> Self::Guard {
        disable_preempt()
    }
    fn read_guard() -> Self::Guard {
        disable_preempt()
    }
}

/// A guardian that disables IRQs while holding a lock.
///
/// This guardian would incur a certain time overhead over
/// [`PreemptDisabled`]. So prefer avoiding using this guardian when
/// IRQ handlers are allowed to get executed while holding the
/// lock. For example, if a lock is never used in the interrupt
/// context, then it is ok not to use this guardian in the process context.
pub struct LocalIrqDisabled;

impl Guardian for LocalIrqDisabled {
    type Guard = DisabledLocalIrqGuard;
    type ReadGuard = DisabledLocalIrqGuard;

    fn guard() -> Self::Guard {
        disable_local()
    }
    fn read_guard() -> Self::Guard {
        disable_local()
    }
}

/// A guardian that disables IRQs while holding a write lock.
///
/// This guardian should only be used for a [`RwLock`]. Using it with a [`SpinLock`] will behave in
/// the same way as using [`LocalIrqDisabled`].
///
/// When using this guardian with a [`RwLock`], holding the read lock will only disable preemption,
/// but holding a write lock will disable local IRQs. The user must ensure that the IRQ handlers
/// never take the write lock, so we can take the read lock without disabling IRQs, but we are
/// still free of deadlock even if the IRQ handlers are triggered in the middle.
///
/// [`RwLock`]: super::RwLock
/// [`SpinLock`]: super::SpinLock
pub struct WriteIrqDisabled;

impl Guardian for WriteIrqDisabled {
    type Guard = DisabledLocalIrqGuard;
    type ReadGuard = DisabledPreemptGuard;

    fn guard() -> Self::Guard {
        disable_local()
    }
    fn read_guard() -> Self::ReadGuard {
        disable_preempt()
    }
}
