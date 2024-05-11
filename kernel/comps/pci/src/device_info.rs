// SPDX-License-Identifier: MPL-2.0

use aster_frame::arch::pci::PciDeviceLocation;

use super::cfg_space::PciDeviceCommonCfgOffset;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct PciDeviceId {
    pub vendor_id: u16,
    pub device_id: u16,
    pub revision_id: u8,
    pub prog_if: u8,
    pub subclass: u8,
    pub class: u8,
    pub subsystem_vendor_id: u16,
    pub subsystem_id: u16,
}

impl PciDeviceId {
    pub(super) fn new(location: PciDeviceLocation) -> Self {
        let vendor_id = location.read16(PciDeviceCommonCfgOffset::VendorId as u16);
        let device_id = location.read16(PciDeviceCommonCfgOffset::DeviceId as u16);
        let revision_id = location.read8(PciDeviceCommonCfgOffset::RevisionId as u16);
        let prog_if = location.read8(PciDeviceCommonCfgOffset::ClassCode as u16);
        let subclass = location.read8(PciDeviceCommonCfgOffset::ClassCode as u16 + 1);
        let class = location.read8(PciDeviceCommonCfgOffset::ClassCode as u16 + 1);
        let subsystem_vendor_id =
            location.read16(PciDeviceCommonCfgOffset::SubsystemVendorId as u16);
        let subsystem_id = location.read16(PciDeviceCommonCfgOffset::SubsystemId as u16);
        Self {
            vendor_id,
            device_id,
            revision_id,
            prog_if,
            subclass,
            class,
            subsystem_vendor_id,
            subsystem_id,
        }
    }
}
