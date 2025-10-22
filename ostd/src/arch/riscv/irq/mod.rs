// SPDX-License-Identifier: MPL-2.0

//! Interrupts.

pub(super) mod chip;
mod ipi;
mod ops;
mod remapping;

pub use chip::{InterruptSourceInFdt, IrqChip, MappedIrqLine, IRQ_CHIP};
pub(crate) use ipi::{send_ipi, HwCpuId};
pub(crate) use ops::{disable_local, enable_local, enable_local_and_halt, is_local_enabled};
pub(crate) use remapping::IrqRemapping;

use crate::{arch::irq::chip::InterruptSourceOnChip, cpu::CpuId};

pub(crate) const IRQ_NUM_MIN: u8 = 0;
pub(crate) const IRQ_NUM_MAX: u8 = 255;

/// An IRQ line with additional information that helps acknowledge the interrupt
/// on hardware.
///
/// On RISC-V, it's the software that routes the interrupt to the IRQ line.
/// Therefore, the software needs to maintain interrupt source information that
/// bridges between software abstraction (e.g., `IRQ_CHIP`) and hardware
/// mechanism (e.g., PLIC).
pub(crate) struct HwIrqLine {
    irq_num: u8,
    source: InterruptSource,
}

pub(super) enum InterruptSource {
    Timer,
    #[expect(private_interfaces)]
    External(InterruptSourceOnChip),
    Software,
}

impl HwIrqLine {
    pub(super) fn new(irq_num: u8, source: InterruptSource) -> Self {
        Self { irq_num, source }
    }

    pub(crate) fn irq_num(&self) -> u8 {
        self.irq_num
    }

    pub(crate) fn ack(&self) {
        match &self.source {
            InterruptSource::Timer => {}
            InterruptSource::External(interrupt_source_on_chip) => {
                IRQ_CHIP.get().unwrap().complete_interrupt(
                    // No races because we are in IRQs.
                    CpuId::current_racy().as_usize() as u32,
                    *interrupt_source_on_chip,
                );
            }
            InterruptSource::Software => unimplemented!(),
        }
    }
}
