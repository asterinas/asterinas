// SPDX-License-Identifier: MPL-2.0

//! PCI device Information

use core::iter;

use super::cfg_space::PciDeviceCommonCfgOffset;

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
}

impl PciDeviceLocation {
    const BIT32_ALIGN_MASK: u16 = 0xFFFC;

    /// Aligns the given pointer to a 32-bit boundary.
    pub const fn align_ptr(ptr: u16) -> u16 {
        ptr & Self::BIT32_ALIGN_MASK
    }

    /// Reads an 8-bit value from the PCI configuration space at the specified offset.
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
        crate::arch::read32(self, offset as u32).unwrap()
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

    /// Writes a 16-bit value to the PCI configuration space at the specified offset.
    pub fn write16(&self, offset: u16, val: u16) {
        let old = self.read32(offset & Self::BIT32_ALIGN_MASK);
        let dest = (offset as usize & 0b10) << 3;
        let mask = (0xFFFF << dest) as u32;
        self.write32(
            offset & Self::BIT32_ALIGN_MASK,
            (((val as u32) << dest) | (old & !mask)).to_le(),
        );
    }

    /// Writes a 32-bit value to the PCI configuration space at the specified offset.
    pub fn write32(&self, offset: u16, val: u32) {
        debug_assert!(
            (offset & 0b11) == 0,
            "misaligned PCI configuration dword u32 write"
        );
        crate::arch::write32(self, offset as u32, val).unwrap()
    }
}

/// PCI device ID
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct PciDeviceId {
    /// Vendor ID
    pub vendor_id: u16,
    /// Device ID
    pub device_id: u16,
    /// Revision ID
    pub revision_id: u8,
    /// Programming Interface Byte
    pub prog_if: u8,
    /// Specifies the specific function the device performs.
    pub subclass: u8,
    /// Specifies the type of function the device performs.
    pub class: u8,
    /// Subsystem Vendor ID
    pub subsystem_vendor_id: u16,
    /// Subsystem ID
    pub subsystem_id: u16,
}

impl PciDeviceId {
    pub(super) fn new(location: PciDeviceLocation) -> Self {
        let vendor_id = location.read16(PciDeviceCommonCfgOffset::VendorId as u16);
        let device_id = location.read16(PciDeviceCommonCfgOffset::DeviceId as u16);
        let revision_id = location.read8(PciDeviceCommonCfgOffset::RevisionId as u16);
        let prog_if = location.read8(PciDeviceCommonCfgOffset::ClassCode as u16);
        let subclass = location.read8(PciDeviceCommonCfgOffset::ClassCode as u16 + 1);
        let class = location.read8(PciDeviceCommonCfgOffset::ClassCode as u16 + 2);
        let subsystem_vendor_id =
            location.read16(PciDeviceCommonCfgOffset::SubsystemVendorId as u16);
        let subsystem_id = location.read16(PciDeviceCommonCfgOffset::SubsystemId as u16);
        Self {
            vendor_id,
            device_id,
            revision_id,
            prog_if,
            subclass,
            class,
            subsystem_vendor_id,
            subsystem_id,
        }
    }
}
