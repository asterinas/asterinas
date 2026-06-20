// SPDX-License-Identifier: MPL-2.0

//! PCI bus access

use core::ops::RangeInclusive;

use ostd::Error;

use crate::PciDeviceLocation;

pub(crate) fn write32(
    _location: &PciDeviceLocation,
    _offset: u32,
    _value: u32,
) -> Result<(), Error> {
    Err(Error::IoError)
}

pub(crate) fn read32(_location: &PciDeviceLocation, _offset: u32) -> Result<u32, Error> {
    Err(Error::IoError)
}

/// Initializes the platform-specific module for accessing the PCI configuration space.
///
/// Returns a range for the PCI bus number, or [`None`] if there is no PCI bus.
pub(crate) fn init() -> Option<RangeInclusive<u8>> {
    // TODO: Support the PCI bus in ARM.
    None
}

pub(crate) const MSIX_DEFAULT_MSG_ADDR: u32 = 0x2400_0000;

pub(crate) fn construct_remappable_msix_address(_remapping_index: u32) -> u32 {
    unimplemented!()
}
