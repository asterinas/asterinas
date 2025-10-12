// SPDX-License-Identifier: MPL-2.0

use spin::once::Once;

use crate::arch::iommu::{alloc_irt_entry, has_interrupt_remapping, IrtEntryHandle};

pub(crate) struct IrqRemapping {
    entry: Once<IrtEntryHandle>,
}

impl IrqRemapping {
    pub(crate) const fn new() -> Self {
        Self { entry: Once::new() }
    }

    /// Initializes the remapping entry for the specific IRQ number.
    ///
    /// This will do nothing if the entry is already initialized or interrupt
    /// remapping is disabled or not supported by the architecture.
    pub(crate) fn init(&self, irq_num: u8) {
        if !has_interrupt_remapping() {
            return;
        }

        self.entry.call_once(|| {
            // Allocate and enable the IRT entry.
            let handle = alloc_irt_entry().unwrap();
            handle.enable(irq_num as u32);
            handle
        });
    }

    /// Gets the remapping index of the IRQ line.
    ///
    /// This method will return `None` if interrupt remapping is disabled or
    /// not supported by the architecture.
    pub(crate) fn remapping_index(&self) -> Option<u16> {
        Some(self.entry.get()?.index())
    }
}
