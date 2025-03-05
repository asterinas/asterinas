// SPDX-License-Identifier: MPL-2.0

//! CPU context & state control and CPU local memory.

pub mod context;
pub mod local;

/// Halts the CPU.
///
/// This function halts the CPU until the next interrupt is received. By
/// halting, the CPU will enter the C-0 state and consume less power.
///
/// Since the function sleeps the CPU, it should not be used within an atomic
/// mode ([`crate::task::atomic_mode`]).
#[track_caller]
pub fn sleep_for_interrupt() {
    crate::task::atomic_mode::might_sleep();
    x86_64::instructions::hlt();
}
