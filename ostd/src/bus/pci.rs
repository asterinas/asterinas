// SPDX-License-Identifier: MPL-2.0

//! Helper functions or structures for PCI devices.

use core::iter;

/// Checks if the system has a PCI bus.
pub fn has_pci_bus() -> bool {
    crate::arch::pci::has_pci_bus()
}

#[cfg(target_arch = "loongarch64")]
/// Allocates an MMIO address range using the global allocator.
pub fn alloc_mmio(layout: core::alloc::Layout) -> Option<crate::prelude::Paddr> {
    crate::arch::pci::alloc_mmio(layout)
}

/// PCI device Location
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PciDeviceLocation {
    /// Bus number
    pub bus: u8,
    /// Device number with max 31
    pub device: u8,
    /// Device number with max 7
    pub function: u8,
}

impl PciDeviceLocation {
    // TODO: Find a proper way to obtain the bus range. For example, if the PCI bus is identified
    // from a device tree, this information can be obtained from the `bus-range` field (e.g.,
    // `bus-range = <0x00 0x7f>`).
    const MIN_BUS: u8 = 0;
    #[cfg(not(target_arch = "loongarch64"))]
    const MAX_BUS: u8 = 255;
    #[cfg(target_arch = "loongarch64")]
    const MAX_BUS: u8 = 127;

    const MIN_DEVICE: u8 = 0;
    const MAX_DEVICE: u8 = 31;

    const MIN_FUNCTION: u8 = 0;
    const MAX_FUNCTION: u8 = 7;

    /// Returns an iterator that enumerates all possible PCI device locations.
    pub fn all() -> impl Iterator<Item = PciDeviceLocation> {
        iter::from_coroutine(
            #[coroutine]
            || {
                for bus in Self::MIN_BUS..=Self::MAX_BUS {
                    for device in Self::MIN_DEVICE..=Self::MAX_DEVICE {
                        for function in Self::MIN_FUNCTION..=Self::MAX_FUNCTION {
                            let loc = PciDeviceLocation {
                                bus,
                                device,
                                function,
                            };
                            yield loc;
                        }
                    }
                }
            },
        )
    }

    /// The page table of all devices is the same. So we can use any device ID.
    /// FIXME: distinguish different device id.
    pub fn zero() -> Self {
        Self {
            bus: 0,
            device: 0,
            function: 0,
        }
    }
}

impl PciDeviceLocation {
    const BIT32_ALIGN_MASK: u16 = 0xFFFC;

    /// Reads a 8-bit value from the PCI configuration space at the specified offset.
    pub fn read8(&self, offset: u16) -> u8 {
        let val = self.read32(offset & Self::BIT32_ALIGN_MASK);
        ((val >> ((offset as usize & 0b11) << 3)) & 0xFF) as u8
    }

    /// Reads a 16-bit value from the PCI configuration space at the specified offset.
    pub fn read16(&self, offset: u16) -> u16 {
        let val = self.read32(offset & Self::BIT32_ALIGN_MASK);
        ((val >> ((offset as usize & 0b10) << 3)) & 0xFFFF) as u16
    }

    /// Reads a 32-bit value from the PCI configuration space at the specified offset.
    pub fn read32(&self, offset: u16) -> u32 {
        debug_assert!(
            (offset & 0b11) == 0,
            "misaligned PCI configuration dword u32 read"
        );
        crate::arch::pci::read32(self, offset as u32).unwrap()
    }

    /// Writes an 8-bit value to the PCI configuration space at the specified offset.
    pub fn write8(&self, offset: u16, val: u8) {
        let old = self.read32(offset & Self::BIT32_ALIGN_MASK);
        let dest = (offset as usize & 0b11) << 3;
        let mask = (0xFF << dest) as u32;
        self.write32(
            offset & Self::BIT32_ALIGN_MASK,
            (((val as u32) << dest) | (old & !mask)).to_le(),
        );
    }

    /// Writes an 16-bit value to the PCI configuration space at the specified offset.
    pub fn write16(&self, offset: u16, val: u16) {
        let old = self.read32(offset & Self::BIT32_ALIGN_MASK);
        let dest = (offset as usize & 0b10) << 3;
        let mask = (0xFFFF << dest) as u32;
        self.write32(
            offset & Self::BIT32_ALIGN_MASK,
            (((val as u32) << dest) | (old & !mask)).to_le(),
        );
    }

    /// Writes an 32-bit value to the PCI configuration space at the specified offset.
    pub fn write32(&self, offset: u16, val: u32) {
        debug_assert!(
            (offset & 0b11) == 0,
            "misaligned PCI configuration dword u32 write"
        );
        crate::arch::pci::write32(self, offset as u32, val).unwrap()
    }
}
