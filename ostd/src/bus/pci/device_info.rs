// SPDX-License-Identifier: MPL-2.0

//! PCI device Information

use super::cfg_space::access::PciDeviceLocation;

/// PCI device ID
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct PciDeviceInfo {
    /// Vendor ID
    pub vendor_id: u16,
    /// Device ID
    pub device_id: u16,
    /// Revision ID
    pub revision_id: u8,
    /// Programming Interface Byte
    pub prog_if: u8,
    /// Specifies the specific function the device performs.
    pub subclass: u8,
    /// Specifies the type of function the device performs.
    pub class_code: u8,
    /// Subsystem Vendor ID
    pub subsystem_vendor_id: u16,
    /// Subsystem ID
    pub subsystem_id: u16,
}

impl PciDeviceInfo {
    pub(super) fn new(location: &PciDeviceLocation) -> Self {
        let vendor_id = location.read_vendor_id().unwrap();
        let device_id = location.read_device_id().unwrap();
        let revision_id = location.read_revision_id().unwrap();
        let prog_if = location.read_prog_if().unwrap();
        let subclass = location.read_subclass().unwrap();
        let class_code = location.read_class_code().unwrap();
        let subsystem_vendor_id = location.read_subsystem_vendor_id().unwrap();
        let subsystem_id = location.read_subsystem_id().unwrap();
        Self {
            vendor_id,
            device_id,
            revision_id,
            prog_if,
            subclass,
            class_code,
            subsystem_vendor_id,
            subsystem_id,
        }
    }
}
