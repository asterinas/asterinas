// SPDX-License-Identifier: MPL-2.0

use crate::bus::pci::{common_device::PciCommonDevice, device_info::PciDeviceLocation};
use crate::{Error, Result};

/// Vendor specific capability. Users can access this capability area at will,
/// except for the PCI configuration space which cannot be accessed at will through this structure.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct CapabilityVndrData {
    location: PciDeviceLocation,
    cap_ptr: u16,
    length: u16,
}

impl CapabilityVndrData {
    pub(super) fn new(dev: &PciCommonDevice, cap_ptr: u16, length: u16) -> Self {
        Self {
            location: *dev.location(),
            cap_ptr,
            length,
        }
    }

    pub fn len(&self) -> u16 {
        self.length
    }

    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    pub fn read8(&self, offset: u16) -> Result<u8> {
        self.check_range(offset)?;
        Ok(self.location.read8(self.cap_ptr + offset))
    }

    pub fn write8(&self, offset: u16, value: u8) -> Result<()> {
        self.check_range(offset)?;
        self.location.write8(self.cap_ptr + offset, value);
        Ok(())
    }

    pub fn read16(&self, offset: u16) -> Result<u16> {
        self.check_range(offset)?;
        Ok(self.location.read16(self.cap_ptr + offset))
    }

    pub fn write16(&self, offset: u16, value: u16) -> Result<()> {
        self.check_range(offset)?;
        self.location.write16(self.cap_ptr + offset, value);
        Ok(())
    }

    pub fn read32(&self, offset: u16) -> Result<u32> {
        self.check_range(offset)?;
        Ok(self.location.read32(self.cap_ptr + offset))
    }

    pub fn write32(&self, offset: u16, value: u32) -> Result<()> {
        self.check_range(offset)?;
        self.location.write32(self.cap_ptr + offset, value);
        Ok(())
    }

    #[inline]
    fn check_range(&self, offset: u16) -> Result<()> {
        if self.length < offset {
            return Err(Error::InvalidArgs);
        }
        Ok(())
    }
}
