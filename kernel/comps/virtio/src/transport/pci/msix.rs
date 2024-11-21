// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;

use ostd::{bus::pci::capability::msix::CapabilityMsixData, trap::IrqLine};

pub struct VirtioMsixManager {
    config_msix_vector: u16,
    /// Shared interrupt vector used by queue.
    shared_interrupt_vector: u16,
    /// The MSI-X vectors allocated to queue interrupt except `shared_interrupt_vector`. All the
    /// vector are considered to be occupied by only one queue.
    unused_msix_vectors: Vec<u16>,
    /// Used MSI-X vectors.
    used_msix_vectors: Vec<u16>,
    msix: CapabilityMsixData,
}

impl VirtioMsixManager {
    pub fn new(mut msix: CapabilityMsixData) -> Self {
        let mut msix_vector_list: Vec<u16> = (0..msix.table_size()).collect();
        for i in msix_vector_list.iter() {
            let irq = ostd::trap::IrqLine::alloc().unwrap();
            msix.set_interrupt_vector(irq, *i);
        }
        let config_msix_vector = msix_vector_list.pop().unwrap();
        let shared_interrupt_vector = msix_vector_list.pop().unwrap();
        Self {
            config_msix_vector,
            unused_msix_vectors: msix_vector_list,
            msix,
            shared_interrupt_vector,
            used_msix_vectors: Vec::new(),
        }
    }

    /// Get config space change MSI-X IRQ, this function will return the MSI-X vector and corresponding IRQ.
    pub fn config_msix_irq(&mut self) -> (u16, &mut IrqLine) {
        (
            self.config_msix_vector,
            self.msix.irq_mut(self.config_msix_vector as usize).unwrap(),
        )
    }

    /// Get shared IRQ line used by virtqueue. If a virtqueue will not send interrupt frequently.
    /// Then this virtqueue should use shared interrupt IRQ.
    /// This function will return the MSI-X vector and corresponding IRQ.
    pub fn shared_irq_line(&mut self) -> (u16, &mut IrqLine) {
        (
            self.shared_interrupt_vector,
            self.msix
                .irq_mut(self.shared_interrupt_vector as usize)
                .unwrap(),
        )
    }

    /// Pop unused vector. If a virtqueue will send interrupt frequently.
    /// Then this virtqueue should use the single IRQ that this function provides.
    /// this function will return the MSI-X vector and corresponding IRQ.
    pub fn pop_unused_irq(&mut self) -> Option<(u16, &mut IrqLine)> {
        let vector = self.unused_msix_vectors.pop()?;
        self.used_msix_vectors.push(vector);
        Some((vector, self.msix.irq_mut(vector as usize).unwrap()))
    }

    /// Returns true if MSI-X is enabled.
    pub fn is_enabled(&self) -> bool {
        self.msix.is_enabled()
    }
}
