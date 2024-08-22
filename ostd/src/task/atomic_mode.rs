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
