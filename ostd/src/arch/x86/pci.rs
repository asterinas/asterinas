// SPDX-License-Identifier: MPL-2.0

//! PCI bus access

use super::device::io_port::{IoPort, ReadWriteAccess, WriteOnlyAccess};
use crate::{bus::pci::PciDeviceLocation, prelude::*};

static PCI_ADDRESS_PORT: IoPort<u32, WriteOnlyAccess> = unsafe { IoPort::new(0x0CF8) };
static PCI_DATA_PORT: IoPort<u32, ReadWriteAccess> = unsafe { IoPort::new(0x0CFC) };

const BIT32_ALIGN_MASK: u32 = 0xFFFC;

pub fn write32(location: &PciDeviceLocation, offset: u32, value: u32) -> Result<()> {
    PCI_ADDRESS_PORT.write(location.encode_as_x86_port_value() | (offset & BIT32_ALIGN_MASK));
    PCI_DATA_PORT.write(value.to_le());
    Ok(())
}

pub fn read32(location: &PciDeviceLocation, offset: u32) -> Result<u32> {
    PCI_ADDRESS_PORT.write(location.encode_as_x86_port_value() | (offset & BIT32_ALIGN_MASK));
    Ok(PCI_DATA_PORT.read().to_le())
}

pub fn has_pci_bus() -> bool {
    true
}
