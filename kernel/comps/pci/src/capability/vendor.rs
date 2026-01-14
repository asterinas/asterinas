// SPDX-License-Identifier: MPL-2.0

//! Vendor-specific capability support.

use ostd::{Error, Result};

use crate::{PciDeviceLocation, common_device::PciCommonDevice};

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

    /// Returns the length of this capability.
    #[expect(clippy::len_without_is_empty)]
    pub fn len(&self) -> u16 {
        self.length
    }

    /// Reads a `u8` from the capability.
    pub fn read8(&self, offset: u16) -> Result<u8> {
        self.check_range(offset, size_of::<u8>() as u16)?;
        Ok(self.location.read8(self.cap_ptr + offset))
    }

    /// Writes a `u8` to the capability.
    pub fn write8(&self, offset: u16, value: u8) -> Result<()> {
        self.check_range(offset, size_of::<u8>() as u16)?;
        self.location.write8(self.cap_ptr + offset, value);
        Ok(())
    }

    /// Reads a `u16` from the capability.
    pub fn read16(&self, offset: u16) -> Result<u16> {
        self.check_range(offset, size_of::<u16>() as u16)?;
        Ok(self.location.read16(self.cap_ptr + offset))
    }

    /// Writes a `u16` to the capability.
    pub fn write16(&self, offset: u16, value: u16) -> Result<()> {
        self.check_range(offset, size_of::<u16>() as u16)?;
        self.location.write16(self.cap_ptr + offset, value);
        Ok(())
    }

    /// Reads a `u32` from the capability.
    pub fn read32(&self, offset: u16) -> Result<u32> {
        self.check_range(offset, size_of::<u32>() as u16)?;
        Ok(self.location.read32(self.cap_ptr + offset))
    }

    /// Writes a `u32` to the capability.
    pub fn write32(&self, offset: u16, value: u32) -> Result<()> {
        self.check_range(offset, size_of::<u32>() as u16)?;
        self.location.write32(self.cap_ptr + offset, value);
        Ok(())
    }

    fn check_range(&self, offset: u16, size: u16) -> Result<()> {
        if let Some(end) = offset.checked_add(size)
            && end <= self.length
        {
            return Ok(());
        }
        Err(Error::InvalidArgs)
    }
}
