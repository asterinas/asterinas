// SPDX-License-Identifier: MPL-2.0

//! Interrupts.

pub(super) mod chip;
pub(super) mod ipi;
mod ops;
mod remapping;

pub use chip::{IRQ_CHIP, InterruptSourceInFdt, IrqChip, MappedIrqLine};
pub(crate) use ipi::{HwCpuId, send_ipi};
pub(crate) use ops::{
    disable_local, disable_local_and_halt, enable_local, enable_local_and_halt, is_local_enabled,
};
pub(crate) use remapping::IrqRemapping;

use crate::arch::irq::chip::InterruptSourceOnChip;

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
                    crate::arch::boot::smp::get_current_hart_id(),
                    *interrupt_source_on_chip,
                );
            }
            InterruptSource::Software => {
                // SAFETY: We have already handled the IPI. So clearing the
                // software interrupt pending bit is safe.
                unsafe { riscv::register::sip::clear_ssoft() };
            }
        }
    }
}
