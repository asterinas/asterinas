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
/// On LoongArch, timer interrupts are acknowledged through `TICLR`, while
/// external interrupts are acknowledged through the extended I/O interrupt
/// controller.
pub(crate) struct HwIrqLine {
    irq_num: u8,
    source: InterruptSource,
}

pub(super) enum InterruptSource {
    Timer,
    External,
}

impl HwIrqLine {
    pub(super) fn new(irq_num: u8, source: InterruptSource) -> Self {
        Self { irq_num, source }
    }

    pub(crate) fn irq_num(&self) -> u8 {
        self.irq_num
    }

    pub(crate) fn ack(&self) {
        match self.source {
            InterruptSource::Timer => loongArch64::register::ticlr::clear_timer_interrupt(),
            InterruptSource::External => chip::complete(self.irq_num),
        }
    }
}
