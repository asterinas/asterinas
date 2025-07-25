// SPDX-License-Identifier: MPL-2.0

//! PCI bus access

use super::device::io_port::{ReadWriteAccess, WriteOnlyAccess};
use crate::{bus::pci::PciDeviceLocation, io::IoPort, prelude::*};

static PCI_ADDRESS_PORT: IoPort<u32, WriteOnlyAccess> = unsafe { IoPort::new(0x0CF8) };
static PCI_DATA_PORT: IoPort<u32, ReadWriteAccess> = unsafe { IoPort::new(0x0CFC) };

const BIT32_ALIGN_MASK: u32 = 0xFFFC;

pub(crate) fn write32(location: &PciDeviceLocation, offset: u32, value: u32) -> Result<()> {
    PCI_ADDRESS_PORT.write(encode_as_port(location) | (offset & BIT32_ALIGN_MASK));
    PCI_DATA_PORT.write(value.to_le());
    Ok(())
}

pub(crate) fn read32(location: &PciDeviceLocation, offset: u32) -> Result<u32> {
    PCI_ADDRESS_PORT.write(encode_as_port(location) | (offset & BIT32_ALIGN_MASK));
    Ok(PCI_DATA_PORT.read().to_le())
}

/// Encodes the bus, device, and function into a port address for use with the PCI I/O port.
fn encode_as_port(location: &PciDeviceLocation) -> u32 {
    // 1 << 31: Configuration enable
    (1 << 31)
        | ((location.bus as u32) << 16)
        | (((location.device as u32) & 0b11111) << 11)
        | (((location.function as u32) & 0b111) << 8)
}

pub(crate) fn has_pci_bus() -> bool {
    true
}
