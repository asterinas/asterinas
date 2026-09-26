// SPDX-License-Identifier: MPL-2.0

//! Interrupts.

pub(super) mod chip;
mod ipi;
mod ops;
mod remapping;

pub use chip::{IRQ_CHIP, InterruptSourceInFdt, IrqChip, MappedIrqLine};
pub(crate) use ipi::{HwCpuId, send_ipi};
pub(super) use ops::PSTATE_I;
pub(crate) use ops::{
    disable_local, disable_local_and_halt, enable_local, enable_local_and_halt, is_local_enabled,
};
pub(crate) use remapping::IrqRemapping;

pub(crate) const IRQ_NUM_MIN: u8 = 0;
pub(crate) const IRQ_NUM_MAX: u8 = 254;
const IRQ_NUM_INVALID: u8 = 255;

/// An IRQ line with additional information that helps acknowledge the interrupt
/// on hardware.
///
/// On ARM, it's the software that routes the interrupt to the IRQ line.
/// Therefore, the software needs to maintain interrupt source information that
/// bridges between software abstraction (e.g., `IRQ_CHIP`) and hardware
/// mechanism (e.g., GIC).
pub(crate) struct HwIrqLine {
    irq_num: u8,
    source: chip::InterruptSourceOnChip,
}

impl HwIrqLine {
    pub(crate) fn irq_num(&self) -> u8 {
        self.irq_num
    }

    pub(crate) fn ack(&self) {
        IRQ_CHIP.get().unwrap().complete_interrupt(self.source);
    }
}
