// SPDX-License-Identifier: MPL-2.0

//! PCI bus io port

use super::device::io_port::{IoPort, ReadWriteAccess, WriteOnlyAccess};
use crate::bus::pci::{common_device::PciCommonDevice, PciDeviceLocation, PCI_BUS};

pub static PCI_ADDRESS_PORT: IoPort<u32, WriteOnlyAccess> = unsafe { IoPort::new(0x0CF8) };
pub static PCI_DATA_PORT: IoPort<u32, ReadWriteAccess> = unsafe { IoPort::new(0x0CFC) };

pub(crate) fn init() {
    let mut lock = PCI_BUS.lock();
    for location in PciDeviceLocation::all() {
        let Some(device) = PciCommonDevice::new(location) else {
            continue;
        };
        lock.register_common_device(device);
    }
}
