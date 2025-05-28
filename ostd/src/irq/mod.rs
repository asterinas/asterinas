// SPDX-License-Identifier: MPL-2.0

//! Handling of Interrupt ReQuests (IRQs).
//!
//! # Top vs bottom half
//!
//! OSTD divides the handling of an IRQ into two parts:
//! **top half** and **bottom half**.
//!
//! A driver can assign to a target device an IRQ line, [`IrqLine`],
//! to which a callback function may be registered.
//! When an IRQ arrives at an IRQ line,
//! OSTD will invoke all the callbacks registered on the line,
//! with all local IRQs on the CPU disabled.
//! Thus, the `IrqLine` callbacks should be written as short as possible,
//! performing the most critical tasks.
//! This is the so-called top half of IRQ handling.
//!
//! When the top half finishes,
//! OSTD continues on the handling of the IRQ with the bottom half.
//! The logic of the bottom half is specified
//! by a callback function registered via [`register_bottom_half_handler_l1`]
//! (or [`register_bottom_half_handler_l2`], as we will see later).
//! The implementer of this callback function may re-enable local IRQs,
//! thus allowing the less critical tasks performed in the bottom half
//! to be preempted by the more critical ones done in the top half.
//!
//! OSTD's split of IRQ handling in top and bottom halves
//! closely resembles that of Linux,
//! but with a key difference:
//! OSTD itself does not hardcode any concrete mechanisms for the bottom-half,
//! e.g., Linux's softirqs or tasklets.
//! OSTD's APIs are flexible and powerful enough to
//! enable an OSTD-based kernel to implement such mechanisms itself.
//! This design helps contain the size and complexity of OSTD.
//!
//! # Nested interrupts
//!
//! OSTD allows interrupts to be nested.
//! The top-half for handling nested interrupts are still done by `IrqLine` callbacks,
//! yet the bottom-half logic is done by a new callback
//! registered via [`register_bottom_half_handler_l2`],
//! rather than [`register_bottom_half_handler_l1`].
//!
//! We introduce the concept of **interrupt level** to
//! mark the nesting depth of interrupts.
//! [`InterruptLevel::current`] keeps track of the current nesting depth
//! on the CPU where the code is executing.
//! There are three interrupt levels:
//!
//! - **Level 0 (Task Context):**
//!   Normal execution for a kernel or user task.
//!   Code at this level can be preempted by a hardware interrupt.
//! - **Level 1 (Interrupt Context):**
//!   Entered when an interrupt preempts task context code.
//!   Interrupt handling callbacks that may be invoked at this level are:
//!   - The top-half callbacks registered via [`IrqLine`];
//!   - The bottom-half callback registered via [`register_bottom_half_handler_l1`].
//! - **Level 2 (Nested Interrupt Context):**
//!   The maximum nesting level,
//!   entered when a level 1 bottom-half callback
//!   (registered via `register_bottom_half_handler_l1`) is interrupted.
//!   `IrqLine` callbacks always have IRQ disabled;
//!   thus, they can never be preempted.
//!
//!   Interrupt handling callbacks that may be invoked at this level are:
//!   - The top-half callbacks registered via [`IrqLine`];
//!   - The bottom-half callback registered via [`register_bottom_half_handler_l2`]
//!     (not [`register_bottom_half_handler_l1`]).
//!
//!   At this level, all local IRQs are disabled to prevent further nesting.
//!

mod bottom_half;
mod guard;
mod level;
mod top_half;

pub use bottom_half::{register_bottom_half_handler_l1, register_bottom_half_handler_l2};
pub use guard::{disable_local, DisabledLocalIrqGuard};
pub use level::InterruptLevel;
pub use top_half::{IrqCallbackFunction, IrqLine};

use crate::{
    arch::{irq::HwIrqLine, trap::TrapFrame},
    cpu::PrivilegeLevel,
};

pub(crate) fn call_irq_callback_functions(
    trap_frame: &TrapFrame,
    hw_irq_line: &HwIrqLine,
    cpu_priv_at_irq: PrivilegeLevel,
) {
    level::enter(
        move || {
            top_half::process(trap_frame, hw_irq_line);
            bottom_half::process(hw_irq_line.irq_num());
        },
        cpu_priv_at_irq,
    );
}
