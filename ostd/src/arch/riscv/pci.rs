// SPDX-License-Identifier: MPL-2.0

//! PCI bus access

use log::warn;
use spin::Once;

use super::boot::DEVICE_TREE;
use crate::{bus::pci::PciDeviceLocation, io::IoMem, mm::VmIoOnce, prelude::*, Error};

static PCI_BASE_ADDR: Once<IoMem> = Once::new();

pub(crate) fn write32(location: &PciDeviceLocation, offset: u32, value: u32) -> Result<()> {
    PCI_BASE_ADDR.get().ok_or(Error::IoError)?.write_once(
        (encode_as_address_offset(location) | (offset & 0xfc)) as usize,
        &value,
    )
}

pub(crate) fn read32(location: &PciDeviceLocation, offset: u32) -> Result<u32> {
    PCI_BASE_ADDR
        .get()
        .ok_or(Error::IoError)?
        .read_once((encode_as_address_offset(location) | (offset & 0xfc)) as usize)
}

pub(crate) fn has_pci_bus() -> bool {
    PCI_BASE_ADDR.is_completed()
}

pub(crate) fn init() -> Result<()> {
    let pci = DEVICE_TREE
        .get()
        .unwrap()
        .find_node("/soc/pci")
        .ok_or(Error::IoError)?;

    let mut reg = pci.reg().ok_or(Error::IoError)?;

    let Some(region) = reg.next() else {
        warn!("PCI node should have exactly one `reg` property, but found zero `reg`s");
        return Err(Error::IoError);
    };
    if reg.next().is_some() {
        warn!(
            "PCI node should have exactly one `reg` property, but found {} `reg`s",
            reg.count() + 2
        );
        return Err(Error::IoError);
    }

    PCI_BASE_ADDR.call_once(|| {
        IoMem::acquire(
            (region.starting_address as usize)
                ..(region.starting_address as usize + region.size.unwrap()),
        )
        .unwrap()
    });

    Ok(())
}

pub(crate) const MSIX_DEFAULT_MSG_ADDR: u32 = 0x2400_0000;

pub(crate) fn construct_remappable_msix_address(remapping_index: u32) -> u32 {
    unimplemented!()
}

/// Encodes the bus, device, and function into an address offset in the PCI MMIO region.
fn encode_as_address_offset(location: &PciDeviceLocation) -> u32 {
    ((location.bus as u32) << 16)
        | ((location.device as u32) << 11)
        | ((location.function as u32) << 8)
}
