// SPDX-License-Identifier: MPL-2.0

//! The interrupt level.

use crate::cpu_local_cell;

// Enter the scope of interrupt handling,
// increasing the interrupt level by one.
pub(super) fn enter<F: FnOnce()>(f: F) {
    INTERRUPT_LEVEL.add_assign(1);

    f();

    INTERRUPT_LEVEL.sub_assign(1);
}

/// The current interrupt level on a CPU.
///
/// This type tracks the current nesting depth on the CPU
/// where the code is executing.
/// An `InterruptLevel` is specific to a single CPU
/// and is meaningless when used by or sent to other CPUs,
/// hence it is `!Send` and `!Sync`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum InterruptLevel {
    /// Level 0 (the task context).
    L0 = 0,
    /// Level 1 (the interrupt context).
    L1 = 1,
    /// Level 2 (the interrupt context due to nested interrupts).
    L2 = 2,
}

impl !Send for InterruptLevel {}
impl !Sync for InterruptLevel {}

impl InterruptLevel {
    /// Returns the current interrupt level of this CPU.
    pub fn current() -> Self {
        let level = INTERRUPT_LEVEL.load();
        match level {
            0 => Self::L0,
            1 => Self::L1,
            2 => Self::L2,
            _ => unreachable!("level must between 0 and 2 (inclusive)"),
        }
    }

    /// Checks if the CPU is currently in the task context (level 0).
    pub fn is_task_context(&self) -> bool {
        *self == Self::L0
    }

    /// Checks if the CPU is currently in the interrupt context (level 1 or 2).
    pub fn is_interrupt_context(&self) -> bool {
        *self == Self::L1 || *self == Self::L2
    }
}

cpu_local_cell! {
    static INTERRUPT_LEVEL: u8 = 0;
}
