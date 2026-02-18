// SPDX-License-Identifier: MPL-2.0

//! The interrupt level.

use crate::{cpu::PrivilegeLevel, cpu_local_cell};

/// The current interrupt level on a CPU.
///
/// This type tracks the current nesting depth on the CPU
/// where the code is executing.
/// There are three levels:
/// * Level 0 (the task context);
/// * Level 1 (the interrupt context);
/// * Level 2 (the interrupt context due to nested interrupts).
///
/// An `InterruptLevel` is specific to a single CPU
/// and is meaningless when used by or sent to other CPUs,
/// hence it is `!Send` and `!Sync`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum InterruptLevel {
    /// Level 0 (the task context).
    L0,
    /// Level 1 (the interrupt context).
    ///
    /// The intern value specifies the CPU privilege level of the interrupted code.
    L1(PrivilegeLevel),
    /// Level 2 (the interrupt context due to nested interrupts).
    L2,
}

impl !Send for InterruptLevel {}
impl !Sync for InterruptLevel {}

impl InterruptLevel {
    /// Returns the current interrupt level of this CPU.
    pub fn current() -> Self {
        // Parameters about the encoding of INTERRUPT_LEVEL
        const LEVEL_VAL_OFFSET: u8 = 1;
        const CPU_PRIV_MASK: u8 = 1 << 0;

        let raw_level = INTERRUPT_LEVEL.load();
        let level = raw_level >> LEVEL_VAL_OFFSET;
        match level {
            0 => Self::L0,
            1 => {
                let cpu_priv_at_irq = if (raw_level & CPU_PRIV_MASK) == 0 {
                    PrivilegeLevel::Kernel
                } else {
                    PrivilegeLevel::User
                };
                Self::L1(cpu_priv_at_irq)
            }
            2 => Self::L2,
            _ => unreachable!("level must between 0 and 2 (inclusive)"),
        }
    }

    /// Returns the interrupt level as an integer between 0 and 2 (inclusive).
    pub fn as_u8(&self) -> u8 {
        match self {
            Self::L0 => 0,
            Self::L1(_) => 1,
            Self::L2 => 2,
        }
    }

    /// Checks if the CPU is currently in the task context (level 0).
    pub fn is_task_context(&self) -> bool {
        *self == Self::L0
    }

    /// Checks if the CPU is currently in the interrupt context (level 1 or 2).
    pub fn is_interrupt_context(&self) -> bool {
        matches!(self, Self::L1(_) | Self::L2)
    }
}

/// Enters the scope of interrupt handling,
/// increasing the interrupt level by one.
///
/// The `cpu_priv_at_irq` argument specifies the CPU privilege level of
/// the code interrupted by the IRQ.
pub(super) fn enter<F: FnOnce()>(f: F, cpu_priv_at_irq: PrivilegeLevel) {
    let increment = {
        let bit_0 = match cpu_priv_at_irq {
            PrivilegeLevel::Kernel => 0,
            PrivilegeLevel::User => 1,
        };
        let bit_1 = 0b10;
        bit_1 | bit_0
    };
    INTERRUPT_LEVEL.add_assign(increment);

    f();

    INTERRUPT_LEVEL.sub_assign(increment);
}

cpu_local_cell! {
    /// The interrupt level of the current IRQ.
    ///
    /// We pack two pieces of information into a single byte:
    /// 1. The current interrupt level (bit 1 - 7);
    /// 2. The CPU privilege level of the code interrupted by the IRQ (bit 0).
    ///
    /// More specifically,
    /// the encoding of this byte is summarized in the table below.
    ///
    /// | Values   | Meaning             |
    /// |----------|---------------------|
    /// | `0b00_0` | L0                  |
    /// | `0b01_0` | L1 from kernel      |
    /// | `0b01_1` | L1 from user        |
    /// | `0b10_0` | L2 (L1 from kernel) |
    /// | `0b10_1` | L2 (L1 from user)   |
    ///
    /// This compact encoding allows us to update this value
    /// in a single arithmetic operation (see `enter`).
    static INTERRUPT_LEVEL: u8 = 0;
}
