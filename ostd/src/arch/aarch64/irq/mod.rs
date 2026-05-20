// SPDX-License-Identifier: MPL-2.0

//! Interrupts.

pub(crate) mod chip;
pub(super) mod ipi;
pub(super) mod ops;
pub(super) mod remapping;

pub use chip::{IRQ_CHIP, InterruptSourceInFdt, MappedIrqLine, parse_gic_intid_from_fdt};
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
/// On AArch64, the software needs to maintain interrupt source information that
/// bridges between the software abstraction and the hardware mechanism (GICv3).
/// The `InterruptSource` carries the GIC INTID through the IRQ handling path so
/// that `ack()` can perform EOI without a reverse-mapping lookup.
pub(crate) struct HwIrqLine {
    irq_num: u8,
    source: InterruptSource,
}

/// The source of a hardware interrupt.
///
/// Each variant carries the GIC INTID because ARM64 GICv3 requires EOI
/// (`ICC_EOIR1_EL1`) for all interrupts, including Timer and IPI.
pub(super) enum InterruptSource {
    /// Physical timer PPI (e.g., INTID 30 on QEMU virt).
    Timer { intid: u32 },
    /// External interrupt (SPI or mapped PPI) from GIC.
    External(chip::InterruptSourceOnChip),
    /// Software Generated Interrupt (SGI) for IPI.
    Ipi { intid: u32 },
}

impl HwIrqLine {
    pub(super) fn new(irq_num: u8, source: InterruptSource) -> Self {
        Self { irq_num, source }
    }

    pub(crate) fn irq_num(&self) -> u8 {
        self.irq_num
    }

    pub(crate) fn ack(&self) {
        let intid = match &self.source {
            InterruptSource::Timer { intid } => *intid,
            InterruptSource::External(on_chip) => on_chip.intid,
            InterruptSource::Ipi { intid } => *intid,
        };
        chip::complete_with_intid(intid);
    }
}
