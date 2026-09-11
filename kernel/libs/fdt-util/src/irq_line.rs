// SPDX-License-Identifier: MPL-2.0

use fdt::node::FdtNode;
use ostd::{
    arch::irq::{IRQ_CHIP, InterruptSourceInFdt, MappedIrqLine},
    irq::IrqLine,
};

/// An extension trait that provides [`acquire_irq_lines`] for [`FdtNode`].
///
/// [`acquire_irq_lines`]: Self::acquire_irq_lines
pub trait AcquireIrqLines {
    /// Allocates and maps `N` [`MappedIrqLine`]s according to the `interrupts` property of the
    /// device tree node.
    ///
    /// The caller is expected to pass an accurate value of `N`. This method will fail if the node
    /// contains too many or too few interrupt lines in the `interrupts` property.
    ///
    /// This method will also fail if any interrupt lines in the `interrupt` property is invalid or
    /// unavailable.
    fn acquire_irq_lines<const N: usize>(&self) -> Option<[MappedIrqLine; N]>;
}

impl AcquireIrqLines for FdtNode<'_, '_> {
    fn acquire_irq_lines<const N: usize>(&self) -> Option<[MappedIrqLine; N]> {
        const { assert!(N > 0) };

        fn warn_bad_property(name: &str, property: &str, error: &str) {
            ostd::warn!("node '{}': property '{}' is {}", name, property, error);
        }

        let Some(interrupts) = self.property("interrupts") else {
            warn_bad_property(self.name, "interrupts", "missing");
            return None;
        };
        let (interrupts, reminder) = interrupts.value.as_chunks::<{ size_of::<u32>() }>();
        if !reminder.is_empty() {
            warn_bad_property(self.name, "interrupts", "invalid");
            return None;
        }
        let mut interrupts = interrupts.iter().map(|chunk| u32::from_be_bytes(*chunk));

        let Some(interrupt_parent) = self.property("interrupt-parent") else {
            warn_bad_property(self.name, "interrupt-parent", "missing");
            return None;
        };
        let Some(interrupt_parent) = interrupt_parent.as_usize() else {
            warn_bad_property(self.name, "interrupt-parent", "invalid");
            return None;
        };

        let irq_chip = IRQ_CHIP.get().unwrap();
        let next_fn = |i| -> Option<MappedIrqLine> {
            let Ok(irq_line) = IrqLine::alloc() else {
                ostd::warn!("node '{}': {}-th IRQ line allocation failed", self.name, i);
                return None;
            };
            let Ok(interrupt) = interrupts.next_chunk() else {
                ostd::warn!(
                    "node '{}': expect {} interrupts, but found {} interrupts",
                    self.name,
                    N,
                    i
                );
                return None;
            };
            let Ok(mapped_irq_line) = irq_chip.map_fdt_pin_to(
                InterruptSourceInFdt {
                    interrupt_parent: interrupt_parent as u32,
                    arguments: interrupt,
                },
                irq_line,
            ) else {
                ostd::warn!("node '{}': {}-th IRQ line mapping failed", self.name, i);
                return None;
            };
            Some(mapped_irq_line)
        };
        let irq_lines = core::array::try_from_fn(next_fn)?;

        if interrupts.count() != 0 {
            ostd::warn!(
                "node '{}': expect {} interrupts, but found more interrupts",
                self.name,
                N
            );
            return None;
        }

        Some(irq_lines)
    }
}
