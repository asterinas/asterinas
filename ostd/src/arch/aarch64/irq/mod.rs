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

use crate::{arch::trap::TrapFrame, cpu::PrivilegeLevel};

/// The smallest usable IRQ number.
///
/// GIC INTIDs 0-15 are software-generated interrupts reserved for IPIs; private
/// (PPI) and shared (SPI) peripheral interrupts start at 16.
pub(crate) const IRQ_NUM_MIN: u8 = 16;
/// The largest usable IRQ number handled by this port.
pub(crate) const IRQ_NUM_MAX: u8 = 255;

/// An IRQ line with the information needed to acknowledge the interrupt.
pub(crate) struct HwIrqLine {
    irq_num: u8,
}

impl HwIrqLine {
    #[expect(dead_code)]
    pub(super) fn new(irq_num: u8) -> Self {
        Self { irq_num }
    }

    pub(crate) fn irq_num(&self) -> u8 {
        self.irq_num
    }

    pub(crate) fn ack(&self) {
        if let Some(chip) = IRQ_CHIP.get() {
            chip.complete_interrupt(self.irq_num);
        }
    }
}

/// Dispatches an external interrupt to the registered callbacks.
pub(in crate::arch) fn handle_irq(trap_frame: &TrapFrame, priv_level: PrivilegeLevel) {
    let Some(chip) = IRQ_CHIP.get() else {
        return;
    };

    while let Some(irq_num) = chip.claim_interrupt() {
        // Interrupt IDs 0-15 are software-generated interrupts (SGIs). SGI 0 is
        // used for IPIs and is handled directly rather than through an IRQ line.
        if irq_num == ipi::IPI_SGI {
            // SAFETY: This is invoked in response to an IPI.
            unsafe { crate::smp::do_inter_processor_call(trap_frame) };
            chip.complete_interrupt(irq_num);
            continue;
        }

        let hw_irq_line = HwIrqLine { irq_num };
        crate::irq::call_irq_callback_functions(trap_frame, &hw_irq_line, priv_level);
    }
}
