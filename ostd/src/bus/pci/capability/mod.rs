// SPDX-License-Identifier: MPL-2.0

//! PCI device capabilities.

#![allow(dead_code)]

use alloc::vec::Vec;

use self::{msix::CapabilityMsixData, vendor::CapabilityVndrData};
use super::{
    cfg_space::{PciDeviceCommonCfgOffset, Status},
    common_device::PciCommonDevice,
    PciDeviceLocation,
};

pub mod msix;
pub mod vendor;

/// PCI Capability
#[derive(Debug)]
pub struct Capability {
    id: u8,
    /// Pointer to the capability.
    pos: u16,
    /// Next Capability pointer, 0xFC if self is the last one.
    next_ptr: u16,
    /// The length of this Capability
    len: u16,
    cap_data: CapabilityData,
}

/// PCI Capability data.
#[derive(Debug, Clone)]
pub enum CapabilityData {
    /// Id:0x01, Power Management
    Pm,
    /// Id:0x02, Accelerated Graphics Part
    Agp,
    /// Id:0x03, Vital Product Data
    Vpd,
    /// Id:0x04, Slot Identification
    SlotId,
    /// Id:0x05, Message Signalled Interrupts
    Msi,
    /// Id:0x06, CompactPCI HotSwap
    Chswp,
    /// Id:0x07, PCI-X
    PciX,
    /// Id:0x08, HyperTransport
    Hp,
    /// Id:0x09, Vendor-Specific
    Vndr(CapabilityVndrData),
    /// Id:0x0A, Debug port
    Dbg,
    /// Id:0x0B, CompactPCI Central Resource Control
    Ccrc,
    /// Id:0x0C, PCI Standard Hot-Plug Controller
    Shpc,
    /// Id:0x0D, Bridge subsystem vendor/device ID
    Ssvid,
    /// Id:0x0R, AGP Target PCI-PCI bridge
    Agp3,
    /// Id:0x0F, Secure Device
    Secdev,
    /// Id:0x10, PCI Express
    Exp,
    /// Id:0x11, MSI-X
    Msix(CapabilityMsixData),
    /// Id:0x12, SATA Data/Index Conf
    Sata,
    /// Id:0x13, PCI Advanced Features
    Af,
    /// Id:0x14, Enhanced Allocation
    Ea,
    /// Id:?, Unknown
    Unknown(u8),
}

impl Capability {
    /// 0xFC, the top of the capability position.
    const CAPABILITY_TOP: u16 = 0xFC;

    /// Gets the capability data
    pub fn capability_data(&self) -> &CapabilityData {
        &self.cap_data
    }

    /// Gets the capabilities of one device
    pub(super) fn device_capabilities(dev: &mut PciCommonDevice) -> Vec<Self> {
        if !dev.status().contains(Status::CAPABILITIES_LIST) {
            return Vec::new();
        }
        let mut capabilities = Vec::new();
        let mut cap_ptr =
            dev.location()
                .read8(PciDeviceCommonCfgOffset::CapabilitiesPointer as u16) as u16
                & PciDeviceLocation::BIT32_ALIGN_MASK;
        let mut cap_ptr_vec = Vec::new();
        // read all cap_ptr so that it is easy for us to get the length.
        while cap_ptr > 0 {
            cap_ptr_vec.push(cap_ptr);
            cap_ptr =
                dev.location().read8(cap_ptr + 1) as u16 & PciDeviceLocation::BIT32_ALIGN_MASK;
        }
        cap_ptr_vec.sort();
        // Push here so that we can calculate the length of the last capability.
        cap_ptr_vec.push(Self::CAPABILITY_TOP);
        let length = cap_ptr_vec.len();
        for i in 0..length - 1 {
            let cap_ptr = cap_ptr_vec[i];
            let next_ptr = cap_ptr_vec[i + 1];
            let cap_type = dev.location().read8(cap_ptr);
            let data = match cap_type {
                0x01 => CapabilityData::Pm,
                0x02 => CapabilityData::Agp,
                0x03 => CapabilityData::Vpd,
                0x04 => CapabilityData::SlotId,
                0x05 => CapabilityData::Msi,
                0x06 => CapabilityData::Chswp,
                0x07 => CapabilityData::PciX,
                0x08 => CapabilityData::Hp,
                0x09 => {
                    CapabilityData::Vndr(CapabilityVndrData::new(dev, cap_ptr, next_ptr - cap_ptr))
                }
                0x0A => CapabilityData::Dbg,
                0x0B => CapabilityData::Ccrc,
                0x0C => CapabilityData::Shpc,
                0x0D => CapabilityData::Ssvid,
                0x0E => CapabilityData::Agp3,
                0x0F => CapabilityData::Secdev,
                0x10 => CapabilityData::Exp,
                0x11 => CapabilityData::Msix(CapabilityMsixData::new(dev, cap_ptr)),
                0x12 => CapabilityData::Sata,
                0x13 => CapabilityData::Af,
                0x14 => CapabilityData::Ea,
                _ => CapabilityData::Unknown(cap_type),
            };
            capabilities.push(Self {
                id: cap_type,
                pos: cap_ptr,
                next_ptr,
                len: next_ptr - cap_ptr,
                cap_data: data,
            });
        }
        capabilities
    }
}
