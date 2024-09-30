// SPDX-License-Identifier: MPL-2.0

//! PCI bus io port

use spin::Once;

use super::boot::DEVICE_TREE;
use crate::{bus::pci::PciDeviceLocation, io_mem::IoMem, mm::VmIoOnce, prelude::*, Error};

static PCI_BASE_PORT: Once<IoMem> = Once::new();

pub fn write32(location: &PciDeviceLocation, offset: u32, value: u32) -> Result<()> {
    PCI_BASE_PORT.get().ok_or(Error::IoError)?.write_once(
        (location.encode_as_address_value() | (offset & 0xfc)) as usize,
        &value,
    )
}

pub fn read32(location: &PciDeviceLocation, offset: u32) -> Result<u32> {
    PCI_BASE_PORT
        .get()
        .ok_or(Error::IoError)?
        .read_once((location.encode_as_address_value() | (offset & 0xfc)) as usize)
}

pub fn has_pci_bus() -> bool {
    PCI_BASE_PORT.is_completed()
}

pub(crate) fn init() {
    if let Some(pci) = DEVICE_TREE.get().unwrap().find_node("/soc/pci") {
        if let Some(reg) = pci.reg() {
            for region in reg {
                PCI_BASE_PORT.call_once(|| unsafe {
                    IoMem::new(
                        (region.starting_address as usize)
                            ..(region.starting_address as usize + region.size.unwrap()),
                    )
                });
            }
        }
    }
}
