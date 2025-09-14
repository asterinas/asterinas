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
/// # Example
///
/// ```rust
/// use ostd::irq;
///
/// {
///     let _ = irq::disable_local();
///     todo!("do something when irqs are disabled");
/// }
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

// SAFETY: The guard disables local IRQs, which meets the first
// sufficient condition for atomic mode.
unsafe impl InAtomicMode for DisabledLocalIrqGuard {}

impl DisabledLocalIrqGuard {
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
