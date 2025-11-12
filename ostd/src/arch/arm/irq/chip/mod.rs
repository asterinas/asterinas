// SPDX-License-Identifier: MPL-2.0

use core::{
    fmt,
    ops::{Deref, DerefMut},
};

use gicv3::Gic;
use spin::Once;

use super::HwIrqLine;
use crate::{Result, arch::boot::DEVICE_TREE, io::IoMemAllocatorBuilder, irq::IrqLine};

mod gicv3;

/// The [`IrqChip`] singleton.
pub static IRQ_CHIP: Once<IrqChip> = Once::new();

/// Initializes the Generic Interrupt Controller.
pub(in crate::arch) fn init(io_mem_allocator_builder: &mut IoMemAllocatorBuilder) {
    let fdt = DEVICE_TREE.get().unwrap();
    let Some(gic) = Gic::from_fdt(fdt, io_mem_allocator_builder) else {
        crate::error!("No supported GICs found (only GICv3 is supported now)");
        return;
    };
    crate::info!("Found and initialized GICv3");

    IRQ_CHIP.call_once(|| IrqChip { gic });
}

/// An IRQ chip.
///
/// This abstracts the hardware IRQ chips (or IRQ controllers), allowing the bus
/// or device drivers to enable [`IrqLine`]s (via, e.g., [`map_fdt_pin_to`])
/// regardless of the specifics of the IRQ chip.
///
/// In the ARM architecture, the underlying hardware is typically Generic Interrupt
/// Controller (GIC).
///
/// [`map_fdt_pin_to`]: Self::map_fdt_pin_to
pub struct IrqChip {
    gic: Gic,
}

impl IrqChip {
    /// Maps an IRQ pin specified by `interrupt_source_in_fdt` to an IRQ line.
    pub fn map_fdt_pin_to(
        &self,
        interrupt_source_in_fdt: InterruptSourceInFdt,
        irq_line: IrqLine,
    ) -> Result<MappedIrqLine> {
        let interrupt_source_on_chip = self
            .gic
            .map_interrupt_source_to(interrupt_source_in_fdt, &irq_line)?;
        Ok(MappedIrqLine {
            irq_line,
            interrupt_source_on_chip,
        })
    }

    /// Unmaps an IRQ line from the IRQ chip.
    fn unmap_irq_line(&self, mapped_irq_line: &MappedIrqLine) {
        self.gic
            .unmap_interrupt_source(mapped_irq_line.interrupt_source_on_chip);
    }

    /// Claims a pending interrupt.
    pub(in crate::arch) fn claim_interrupt(&self) -> Option<HwIrqLine> {
        self.gic.claim_interrupt()
    }

    /// Completes an active interrupt.
    pub(super) fn complete_interrupt(&self, interrupt_source: InterruptSourceOnChip) {
        self.gic.complete_interrupt(interrupt_source);
    }
}

/// An [`IrqLine`] mapped to an IRQ pin managed by the [`IRQ_CHIP`].
///
/// When the object is dropped, the IRQ line will be unmapped by the IRQ chip.
pub struct MappedIrqLine {
    irq_line: IrqLine,
    interrupt_source_on_chip: InterruptSourceOnChip,
}

impl fmt::Debug for MappedIrqLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MappedIrqLine")
            .field("irq_line", &self.irq_line)
            .field("interrupt_source_on_chip", &self.interrupt_source_on_chip)
            .finish_non_exhaustive()
    }
}

impl Deref for MappedIrqLine {
    type Target = IrqLine;

    fn deref(&self) -> &Self::Target {
        &self.irq_line
    }
}

impl DerefMut for MappedIrqLine {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.irq_line
    }
}

impl Drop for MappedIrqLine {
    fn drop(&mut self) {
        IRQ_CHIP.get().unwrap().unmap_irq_line(self)
    }
}

/// Interrupt source identifier in the device tree.
#[derive(Clone, Copy, Debug)]
pub struct InterruptSourceInFdt {
    /// Phandle of the interrupt controller it connects to.
    pub interrupt_parent: u32,
    /// Arguments (e.g., some index and flags) that describe the interrupt.
    pub arguments: [u32; 3],
}

/// Interrupt source identifier on the `IRQ_CHIP`.
#[derive(Clone, Copy, Debug)]
pub(super) struct InterruptSourceOnChip {
    /// Phandle of the interrupt controller it connects to.
    interrupt_parent: u32,
    /// Interrupt source number on the interrupt controller.
    interrupt: u32,
}
