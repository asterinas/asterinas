// SPDX-License-Identifier: MPL-2.0

//! PCI bus access

use core::ops::RangeInclusive;

use log::warn;
use ostd::{Error, arch::boot::DEVICE_TREE, io::IoMem, mm::VmIoOnce};
use spin::Once;

use crate::PciDeviceLocation;

static PCI_ECAM_CFG_SPACE: Once<IoMem> = Once::new();

pub(crate) fn write32(location: &PciDeviceLocation, offset: u32, value: u32) -> Result<(), Error> {
    if offset > PCI_ECAM_MAX_OFFSET {
        return Err(Error::InvalidArgs);
    }
    PCI_ECAM_CFG_SPACE.get().ok_or(Error::IoError)?.write_once(
        (encode_as_address_offset(location) | offset) as usize,
        &value,
    )
}

pub(crate) fn read32(location: &PciDeviceLocation, offset: u32) -> Result<u32, Error> {
    if offset > PCI_ECAM_MAX_OFFSET {
        return Err(Error::InvalidArgs);
    }
    PCI_ECAM_CFG_SPACE
        .get()
        .ok_or(Error::IoError)?
        .read_once((encode_as_address_offset(location) | offset) as usize)
}

/// The maximum offset in the 12-bit configuration space when using [`encode_as_address_offset`].
const PCI_ECAM_MAX_OFFSET: u32 = 0xffc;

/// Encodes the bus, device, and function into an address offset in the PCI MMIO region.
fn encode_as_address_offset(location: &PciDeviceLocation) -> u32 {
    // We only support ECAM here for RISC-V platforms. Offsets are from
    // <https://www.kernel.org/doc/Documentation/devicetree/bindings/pci/host-generic-pci.txt>.
    ((location.bus as u32) << 20)
        | ((location.device as u32) << 15)
        | ((location.function as u32) << 12)
}

/// Initializes the platform-specific module for accessing the PCI configuration space.
///
/// Returns a range for the PCI bus number, or [`None`] if there is no PCI bus.
pub(crate) fn init() -> Option<RangeInclusive<u8>> {
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
        return None;
    };

    let Some(mut reg) = pci.reg() else {
        warn!("PCI node should have exactly one `reg` property, but found zero `reg`s");
        return None;
    };
    let Some(region) = reg.next() else {
        warn!("PCI node should have exactly one `reg` property, but found zero `reg`s");
        return None;
    };
    if reg.next().is_some() {
        warn!(
            "PCI node should have exactly one `reg` property, but found {} `reg`s",
            reg.count() + 2
        );
        return None;
    }

    let bus_range = if let Some(prop) = pci.property("bus-range") {
        if prop.value.len() != 8 || prop.value[0..3] != [0, 0, 0] || prop.value[4..7] != [0, 0, 0] {
            warn!(
                "PCI node should have a `bus-range` property with two bytes, but found `{:?}`",
                prop.value
            );
            return None;
        }
        if prop.value[3] != 0 {
            // TODO: We don't support this case because the base address corresponds to the first
            // bus. Therefore, an offset must be applied to the bus value in `read32`/`write32`.
            warn!(
                "PCI node with a non-zero bus start `{}` is not supported yet",
                prop.value[3]
            );
            return None;
        }
        Some(prop.value[3]..=prop.value[7])
    } else {
        // "bus-range: Optional property [..] If absent, defaults to <0 255> (i.e. all buses)."
        Some(0..=255)
    };

    let addr_start = region.starting_address as usize;
    let addr_end = addr_start.checked_add(region.size.unwrap()).unwrap();
    PCI_ECAM_CFG_SPACE.call_once(|| IoMem::acquire(addr_start..addr_end).unwrap());

    bus_range
}

pub(crate) const MSIX_DEFAULT_MSG_ADDR: u32 = 0x2400_0000;

pub(crate) fn construct_remappable_msix_address(_remapping_index: u32) -> u32 {
    unimplemented!()
}
