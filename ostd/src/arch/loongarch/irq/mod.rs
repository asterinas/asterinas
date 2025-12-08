// SPDX-License-Identifier: MPL-2.0

//! Interrupts.

pub(super) mod chip;
mod ipi;
mod ops;
mod remapping;

pub(crate) use ipi::{HwCpuId, send_ipi};
pub(crate) use ops::{
    disable_local, disable_local_and_halt, enable_local, enable_local_and_halt, is_local_enabled,
};
pub(crate) use remapping::IrqRemapping;

pub(crate) const IRQ_NUM_MIN: u8 = 0;
pub(crate) const IRQ_NUM_MAX: u8 = 255;

/// An IRQ line with additional information that helps acknowledge the interrupt
/// on hardware.
///
/// On loongarch64, it's the hardware (i.e., the extended I/O interrupt
/// controller) that routes the interrupt to the IRQ line. Therefore, the
/// software does not need to maintain additional information about the original
/// hardware interrupt.
pub(crate) struct HwIrqLine {
    irq_num: u8,
}

impl HwIrqLine {
    pub(super) fn new(irq_num: u8) -> Self {
        Self { irq_num }
    }

    pub(crate) fn irq_num(&self) -> u8 {
        self.irq_num
    }

    pub(crate) fn ack(&self) {
        chip::complete(self.irq_num);
    }
}
