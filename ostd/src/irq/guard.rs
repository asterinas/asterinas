// SPDX-License-Identifier: MPL-2.0

//! The IRQ disabling guard.

use crate::{arch::irq as arch_irq, sync::GuardTransfer, task::atomic_mode::InAtomicMode};

/// Disables all IRQs on the current CPU (i.e., locally).
///
/// This function returns a guard object, which will automatically enable local IRQs again when
/// it is dropped. This function works correctly even when it is called in a _nested_ way.
/// The local IRQs shall only be re-enabled when the most outer guard is dropped.
///
/// This function can play nicely with [`SpinLock`] as the type uses this function internally.
/// One can invoke this function even after acquiring a spin lock. And the reversed order is also ok.
///
/// [`SpinLock`]: crate::sync::SpinLock
///
/// # Examples
///
/// ```rust
/// # fn work_with_irq_disabled() {}
/// #
/// use ostd::irq;
///
/// let guard = irq::disable_local();
/// // Do something with IRQs disabled.
/// work_with_irq_disabled();
/// // Re-enable IRQs if they were previously enabled.
/// drop(guard);
/// ```
pub fn disable_local() -> DisabledLocalIrqGuard {
    DisabledLocalIrqGuard::new()
}

/// A guard for disabled local IRQs.
#[clippy::has_significant_drop]
#[must_use]
#[derive(Debug)]
pub struct DisabledLocalIrqGuard {
    was_enabled: bool,
}

impl !Send for DisabledLocalIrqGuard {}

// Required by `InAtomicMode`. See the safety section of its doc comments.
impl !Sync for DisabledLocalIrqGuard {}

// SAFETY:
// 1. The guard disables local IRQs;
// 2. The guard cannot be moved to or used by other tasks.
unsafe impl InAtomicMode for DisabledLocalIrqGuard {}

impl DisabledLocalIrqGuard {
    /// Creates a guard for already-disabled local IRQs.
    ///
    /// Dropping this guard enables local IRQs.
    ///
    /// # Safety
    ///
    /// Local IRQs must already be disabled.
    /// The caller must ensure that it is safe to enable them when the guard is dropped,
    /// including that no other live guard requires them to remain disabled at that point.
    pub(crate) unsafe fn assume_disabled() -> Self {
        debug_assert!(!arch_irq::is_local_enabled());
        Self { was_enabled: true }
    }

    fn new() -> Self {
        let was_enabled = arch_irq::is_local_enabled();
        if was_enabled {
            arch_irq::disable_local();
        }
        Self { was_enabled }
    }
}

impl GuardTransfer for DisabledLocalIrqGuard {
    fn transfer_to(&mut self) -> Self {
        let was_enabled = self.was_enabled;
        self.was_enabled = false;
        Self { was_enabled }
    }
}

impl Drop for DisabledLocalIrqGuard {
    fn drop(&mut self) {
        if self.was_enabled {
            arch_irq::enable_local();
        }
    }
}

#[cfg(ktest)]
mod test {
    use super::*;
    use crate::prelude::*;

    #[ktest]
    fn assumed_guard_restores_irqs() {
        assert!(arch_irq::is_local_enabled());
        core::mem::forget(disable_local());

        // SAFETY: We just disabled IRQs and forgot the only guard responsible for
        // re-enabling them. No other guard requires IRQs to remain disabled.
        let guard = unsafe { DisabledLocalIrqGuard::assume_disabled() };
        assert!(!arch_irq::is_local_enabled());

        let nested_guard = disable_local();
        drop(nested_guard);
        assert!(!arch_irq::is_local_enabled());

        drop(guard);
        assert!(arch_irq::is_local_enabled());
    }
}
