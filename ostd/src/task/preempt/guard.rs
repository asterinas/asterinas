// SPDX-License-Identifier: MPL-2.0

/// A guard for disable preempt.
#[clippy::has_significant_drop]
#[must_use]
pub struct DisablePreemptGuard {
    // This private field prevents user from constructing values of this type directly.
    _private: (),
}

impl !Send for DisablePreemptGuard {}

impl DisablePreemptGuard {
    fn new() -> Self {
        super::cpu_local::inc_guard_count();
        Self { _private: () }
    }

    /// Transfer this guard to a new guard.
    /// This guard must be dropped after this function.
    pub fn transfer_to(&self) -> Self {
        disable_preempt()
    }
}

impl Drop for DisablePreemptGuard {
    fn drop(&mut self) {
        super::cpu_local::dec_guard_count();
    }
}

/// Disables preemption.
pub fn disable_preempt() -> DisablePreemptGuard {
    DisablePreemptGuard::new()
}
