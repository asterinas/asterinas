// SPDX-License-Identifier: MPL-2.0

//! MSI-X interrupt management for NVMe devices.
//!
//! This module provides MSI-X interrupt vector allocation and management
//! for NVMe controllers, allowing per-queue interrupt handling for better
//! performance and scalability.

use alloc::vec::Vec;

use aster_pci::capability::msix::CapabilityMsixData;
use ostd::irq::IrqLine;

/// MSI-X interrupt manager for NVMe devices.
///
/// NVMe devices can use multiple MSI-X vectors for improved interrupt handling:
/// - Admin queue vector: Used for admin command completions
/// - I/O queue vectors: Each I/O queue can have its own interrupt vector
pub struct NvmeMsixManager {
    /// MSI-X vector for admin queue (queue 0)
    admin_vector: u16,
    /// Available MSI-X vectors for I/O queues
    unused_vectors: Vec<u16>,
    /// MSI-X vectors currently assigned to I/O queues
    used_vectors: Vec<u16>,
    /// MSI-X capability data
    msix: CapabilityMsixData,
}

impl NvmeMsixManager {
    /// Creates a new MSI-X manager and initializes all available interrupt vectors.
    ///
    /// # Arguments
    /// * `msix` - The MSI-X capability data from PCI configuration space
    ///
    /// # Returns
    /// A new `NvmeMsixManager` with all vectors allocated and initialized.
    pub fn new(mut msix: CapabilityMsixData) -> Self {
        let table_size = msix.table_size();

        // Allocate IRQ lines for all MSI-X vectors
        let mut vector_list: Vec<u16> = (0..table_size).collect();
        for i in vector_list.iter() {
            let irq = IrqLine::alloc().unwrap();
            msix.set_interrupt_vector(irq, *i);
        }

        // Reserve the first vector for admin queue
        let admin_vector = vector_list.remove(0);

        Self {
            admin_vector,
            unused_vectors: vector_list,
            used_vectors: Vec::new(),
            msix,
        }
    }

    /// Gets the admin queue MSI-X vector and its IRQ line.
    ///
    /// # Returns
    /// A tuple of (vector_id, irq_line_reference)
    pub fn admin_irq(&mut self) -> (u16, &mut IrqLine) {
        (
            self.admin_vector,
            self.msix.irq_mut(self.admin_vector as usize).unwrap(),
        )
    }

    /// Allocates an MSI-X vector for an I/O queue.
    ///
    /// # Returns
    ///
    /// This method will return the vector ID and a mutable reference to the [`IrqLine`] if a
    /// vector is available. Otherwise, it will return [`None`].
    pub fn alloc_io_queue_irq(&mut self) -> Option<(u16, &mut IrqLine)> {
        let vector = self.unused_vectors.pop()?;
        self.used_vectors.push(vector);
        Some((vector, self.msix.irq_mut(vector as usize).unwrap()))
    }

    /// Returns the total number of MSI-X vectors available.
    pub fn table_size(&self) -> u16 {
        self.msix.table_size()
    }

    /// Returns a mutable reference to the IRQ line for the given vector if any.
    pub fn irq_for_vector_mut(&mut self, vector: u16) -> Option<&mut IrqLine> {
        self.msix.irq_mut(vector as usize)
    }
}
