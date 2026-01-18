// SPDX-License-Identifier: MPL-2.0

//! PCI bus access

use core::ops::RangeInclusive;

use ostd::{
    Error,
    arch::device::io_port::{ReadWriteAccess, WriteOnlyAccess},
    io::IoPort,
    sync::SpinLock,
};
use spin::Once;

use crate::device_info::PciDeviceLocation;

struct AddressAndDataPort {
    address_port: IoPort<u32, WriteOnlyAccess>,
    data_port: IoPort<u32, ReadWriteAccess>,
}

static PCI_PIO_CFG_SPACE: Once<SpinLock<AddressAndDataPort>> = Once::new();

const BIT32_ALIGN_MASK: u32 = 0xFFFC;

pub(crate) fn write32(location: &PciDeviceLocation, offset: u32, value: u32) -> Result<(), Error> {
    let pio = PCI_PIO_CFG_SPACE.get().ok_or(Error::IoError)?.lock();
    pio.address_port
        .write(encode_as_port(location) | (offset & BIT32_ALIGN_MASK));
    pio.data_port.write(value.to_le());
    Ok(())
}

pub(crate) fn read32(location: &PciDeviceLocation, offset: u32) -> Result<u32, Error> {
    let pio = PCI_PIO_CFG_SPACE.get().ok_or(Error::IoError)?.lock();
    pio.address_port
        .write(encode_as_port(location) | (offset & BIT32_ALIGN_MASK));
    Ok(pio.data_port.read().to_le())
}

/// Encodes the bus, device, and function into a port address for use with the PCI I/O port.
fn encode_as_port(location: &PciDeviceLocation) -> u32 {
    // 1 << 31: Configuration enable
    (1 << 31)
        | ((location.bus as u32) << 16)
        | (((location.device as u32) & 0b11111) << 11)
        | (((location.function as u32) & 0b111) << 8)
}

/// Initializes the platform-specific module for accessing the PCI configuration space.
///
/// Returns a range for the PCI bus number, or [`None`] if there is no PCI bus.
pub(crate) fn init() -> Option<RangeInclusive<u8>> {
    // We use `acquire_overlapping` to acquire the port at 0xCF8 because 0xCF9 may be used as a
    // reset control register in the PIIX4. Although the two ports overlap in their I/O range, they
    // serve completely different purposes. See
    // <https://www.intel.com/Assets/PDF/datasheet/290562.pdf>.
    let address_port = IoPort::acquire_overlapping(0xCF8).unwrap();
    let data_port = IoPort::acquire(0xCFC).unwrap();
    PCI_PIO_CFG_SPACE.call_once(move || {
        SpinLock::new(AddressAndDataPort {
            address_port,
            data_port,
        })
    });

    Some(0..=255)
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
