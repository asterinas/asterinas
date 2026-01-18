// SPDX-License-Identifier: MPL-2.0

//! PCI bus access

use core::ops::RangeInclusive;

use ostd::{
    Error,
    arch::{
        device::io_port::{ReadWriteAccess, WriteOnlyAccess},
        kernel::ACPI_INFO,
    },
    io::{IoMem, IoPort},
    mm::VmIoOnce,
    sync::SpinLock,
};
use spin::Once;

use crate::device_info::PciDeviceLocation;

struct AddressAndDataPort {
    address_port: IoPort<u32, WriteOnlyAccess>,
    data_port: IoPort<u32, ReadWriteAccess>,
}

static PCI_PIO_CFG_SPACE: Once<SpinLock<AddressAndDataPort>> = Once::new();

static PCI_ECAM_CFG_SPACE: Once<IoMem> = Once::new();

pub(crate) fn write32(location: &PciDeviceLocation, offset: u32, value: u32) -> Result<(), Error> {
    if let Some(ecam) = PCI_ECAM_CFG_SPACE.get() {
        if offset > PCI_ECAM_MAX_OFFSET {
            return Err(Error::InvalidArgs);
        }
        ecam.write_once(
            (encode_as_address_offset(location) | offset) as usize,
            &value,
        )?;
        return Ok(());
    }

    if let Some(pio_ports) = PCI_PIO_CFG_SPACE.get() {
        if offset > PCI_PIO_MAX_OFFSET {
            return Err(Error::InvalidArgs);
        }
        let pio = pio_ports.lock();
        pio.address_port.write(encode_as_port(location) | offset);
        pio.data_port.write(value.to_le());
        return Ok(());
    }

    Err(Error::IoError)
}

pub(crate) fn read32(location: &PciDeviceLocation, offset: u32) -> Result<u32, Error> {
    if let Some(ecam) = PCI_ECAM_CFG_SPACE.get() {
        if offset > PCI_ECAM_MAX_OFFSET {
            return Err(Error::InvalidArgs);
        }
        return ecam.read_once((encode_as_address_offset(location) | offset) as usize);
    }

    if let Some(pio_ports) = PCI_PIO_CFG_SPACE.get() {
        if offset > PCI_PIO_MAX_OFFSET {
            return Err(Error::InvalidArgs);
        }
        let pio = pio_ports.lock();
        pio.address_port.write(encode_as_port(location) | offset);
        return Ok(pio.data_port.read().to_le());
    }

    Err(Error::IoError)
}

/// The maximum offset in the 12-bit configuration space when using [`encode_as_address_offset`].
const PCI_ECAM_MAX_OFFSET: u32 = 0xffc;

/// Encodes the bus, device, and function into an address offset in the PCI MMIO region.
fn encode_as_address_offset(location: &PciDeviceLocation) -> u32 {
    ((location.bus as u32) << 20)
        | ((location.device as u32) << 15)
        | ((location.function as u32) << 12)
}

/// The maximum offset in the 8-bit configuration space when using [`encode_as_port`].
const PCI_PIO_MAX_OFFSET: u32 = 0xfc;

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
    if let Some(ecam) = ACPI_INFO.get().unwrap().pci_ecam_region.as_ref() {
        let bus_start = ecam.bus_start;
        let bus_end = ecam.bus_end;

        let addr_start = ecam.base_address as usize;
        // Note that the base address always corresponds to the bus number 0, regardless of the
        // actual value of `bus_start`.
        let addr_end = addr_start + (bus_end as usize + 1) * (1 << 20);
        PCI_ECAM_CFG_SPACE.call_once(|| IoMem::acquire(addr_start..addr_end).unwrap());

        return Some(bus_start..=bus_end);
    }

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
