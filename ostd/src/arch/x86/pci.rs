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

pub(crate) const MSIX_DEFAULT_MSG_ADDR: u32 = 0xFEE0_0000;

pub(crate) fn construct_remappable_msix_address(remapping_index: u32) -> u32 {
    // Use remappable format. The bits[4:3] should be always set to 1 according to the manual.
    let mut address = MSIX_DEFAULT_MSG_ADDR | 0b1_1000;

    // Interrupt index[14:0] is on address[19:5] and interrupt index[15] is on address[2].
    address |= (remapping_index & 0x7FFF) << 5;
    address |= (remapping_index & 0x8000) >> 13;

    address
}
