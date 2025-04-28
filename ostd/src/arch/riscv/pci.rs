// SPDX-License-Identifier: MPL-2.0

//! PCI bus access

use log::warn;
use spin::Once;

use super::boot::DEVICE_TREE;
use crate::{
    bus::pci::PciDeviceLocation,
    io::{IoMem, IoMemAllocatorBuilder},
    mm::{CachePolicy, PageFlags, VmIoOnce},
    prelude::*,
    Error,
};

static PCI_ECAM_CFG_SPACE: Once<IoMem> = Once::new();

pub(crate) fn write32(location: &PciDeviceLocation, offset: u32, value: u32) -> Result<()> {
    PCI_ECAM_CFG_SPACE.get().ok_or(Error::IoError)?.write_once(
        (encode_as_address_offset(location) | (offset & 0xfc)) as usize,
        &value,
    )
}

pub(crate) fn read32(location: &PciDeviceLocation, offset: u32) -> Result<u32> {
    PCI_ECAM_CFG_SPACE
        .get()
        .ok_or(Error::IoError)?
        .read_once((encode_as_address_offset(location) | (offset & 0xfc)) as usize)
}

pub(crate) fn has_pci_bus() -> bool {
    PCI_ECAM_CFG_SPACE.is_completed()
}

pub(crate) fn init() {
    // We follow the Linux's PCI device tree to obtain the register information
    // about the PCI bus. See also the specification at
    // <https://www.kernel.org/doc/Documentation/devicetree/bindings/pci/host-generic-pci.txt>.
    //
    // TODO: Support multiple PCIe segment groups instead of assuming only one
    // PCIe segment group is in use.
    let Some(pci) = DEVICE_TREE
        .get()
        .unwrap()
        .find_compatible(&["pci-host-ecam-generic"])
    else {
        warn!("No generic PCI host controller node found in the device tree");
        return;
    };

    let Some(mut reg) = pci.reg() else {
        warn!("PCI node should have exactly one `reg` property, but found zero `reg`s");
        return;
    };
    let Some(region) = reg.next() else {
        warn!("PCI node should have exactly one `reg` property, but found zero `reg`s");
        return;
    };
    if reg.next().is_some() {
        warn!(
            "PCI node should have exactly one `reg` property, but found {} `reg`s",
            reg.count() + 2
        );
        return;
    }

    let addr_start = region.starting_address as usize;
    let addr_end = addr_start.checked_add(region.size.unwrap()).unwrap();
    PCI_ECAM_CFG_SPACE.call_once(|| IoMem::acquire(addr_start..addr_end).unwrap());
}

pub(crate) const MSIX_DEFAULT_MSG_ADDR: u32 = 0x2400_0000;

pub(crate) fn construct_remappable_msix_address(remapping_index: u32) -> u32 {
    unimplemented!()
}

/// Encodes the bus, device, and function into an address offset in the PCI MMIO region.
fn encode_as_address_offset(location: &PciDeviceLocation) -> u32 {
    // We only support ECAM here for RISC-V platforms. Offsets are from
    // <https://www.kernel.org/doc/Documentation/devicetree/bindings/pci/host-generic-pci.txt>.
    ((location.bus as u32) << 20)
        | ((location.device as u32) << 15)
        | ((location.function as u32) << 12)
}
