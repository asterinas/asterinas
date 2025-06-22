// SPDX-License-Identifier: MPL-2.0

//! Atomic Mode
//!
//! Multitasking, while powerful, can sometimes lead to undesirable
//! or catastrophic consequences if being misused.
//! For instance, a user of OSTD might accidentally write an IRQ handler
//! that relies on mutexes,
//! which could attempt to sleep within an interrupt context---something that must be avoided.
//! Another common mistake is
//! acquiring a spinlock in a task context and then attempting to yield or sleep,
//! which can easily lead to deadlocks.
//!
//! To mitigate the risks associated with improper multitasking,
//! we introduce the concept of atomic mode.
//! Kernel code is considered to be running in atomic mode
//! if one of the following conditions is met:
//!
//! 1. Task preemption is disabled, such as when a spinlock is held.
//! 2. Local IRQs are disabled, such as during interrupt context.
//!
//! While in atomic mode,
//! any attempt to perform "sleep-like" actions will trigger a panic:
//!
//! 1. Switching to another task.
//! 2. Switching to user space.
//!
//! This module provides API to detect such "sleep-like" actions.

use core::sync::atomic::Ordering;

/// Marks a function as one that might sleep.
///
/// This function will panic if it is executed in atomic mode.
#[track_caller]
pub fn might_sleep() {
    let preempt_count = super::preempt::cpu_local::get_guard_count();
    let is_local_irq_enabled = crate::arch::irq::is_local_enabled();
    if (preempt_count != 0 || !is_local_irq_enabled)
        && !crate::IN_BOOTSTRAP_CONTEXT.load(Ordering::Relaxed)
    {
        panic!(
            "This function might break atomic mode (preempt_count = {}, is_local_irq_enabled = {})",
            preempt_count, is_local_irq_enabled
        );
    }
}

/// A marker trait for guard types that enforce the atomic mode.
///
/// Key kernel primitives such as `SpinLock` and `Rcu` rely on
/// [the atomic mode](crate::task::atomic_mode) for correctness or soundness.
/// The existence of such a guard guarantees that the current task is executing
/// in the atomic mode.
///
/// It requires [`core::fmt::Debug`] by default to make it easier to derive
/// [`Debug`] for types with `&dyn InAtomicMode`.
///
/// # Safety
///
/// The implementer must ensure that the atomic mode is maintained while
/// the guard type is alive.
pub unsafe trait InAtomicMode: core::fmt::Debug {}

/// Abstracts any type from which one can obtain a reference to an atomic-mode guard.
pub trait AsAtomicModeGuard {
    /// Returns a guard for the atomic mode.
    fn as_atomic_mode_guard(&self) -> &dyn InAtomicMode;
}

impl<G: InAtomicMode> AsAtomicModeGuard for G {
    fn as_atomic_mode_guard(&self) -> &dyn InAtomicMode {
        self
    }
}

impl AsAtomicModeGuard for dyn InAtomicMode + '_ {
    fn as_atomic_mode_guard(&self) -> &dyn InAtomicMode {
        self
    }
}
