//! This mod is used in frame to do device initialization
use crate::util::CSpaceAccessMethod;
use alloc::vec::Vec;
use jinux_frame::bus::pci::PciDeviceLocation;

use self::{
    exp::CapabilityEXPData, msi::CapabilityMSIData, msix::CapabilityMSIXData, pm::CapabilityPMData,
    sata::CapabilitySATAData, vendor::CapabilityVNDRData,
};

use super::PCI_CAP_PTR;

pub mod exp;
pub mod msi;
pub mod msix;
pub mod pm;
pub mod sata;
pub mod vendor;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum CapabilityData {
    /// Id:0x01, Power Management
    PM(CapabilityPMData),
    /// Id:0x02, Accelerated Graphics Part
    AGP,
    /// Id:0x03, Vital Product Data
    VPD,
    /// Id:0x04, Slot Identification
    SLOTID,
    /// Id:0x05, Message Signalled Interrupts
    MSI(CapabilityMSIData),
    /// Id:0x06, CompactPCI HotSwap
    CHSWP,
    /// Id:0x07, PCI-X
    PCIX,
    /// Id:0x08, HyperTransport
    HP,
    /// Id:0x09, Vendor-Specific
    VNDR(CapabilityVNDRData),
    /// Id:0x0A, Debug port
    DBG,
    /// Id:0x0B, CompactPCI Central Resource Control
    CCRC,
    /// Id:0x0C, PCI Standard Hot-Plug Controller
    SHPC,
    /// Id:0x0D, Bridge subsystem vendor/device ID
    SSVID,
    /// Id:0x0R, AGP Target PCI-PCI bridge
    AGP3,
    /// Id:0x0F, Secure Device
    SECDEV,
    /// Id:0x10, PCI Express
    EXP(CapabilityEXPData),
    /// Id:0x11, MSI-X
    MSIX(CapabilityMSIXData),
    /// Id:0x12, SATA Data/Index Conf
    SATA(CapabilitySATAData),
    /// Id:0x13, PCI Advanced Features
    AF,
    /// Id:0x14, Enhanced Allocation
    EA,
    /// Id:?, Unknown
    Unknown(u8),
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Capability {
    pub cap_ptr: u16,
    pub data: CapabilityData,
}

impl Capability {
    /// get the capabilities of one device
    pub fn device_capabilities(loc: PciDeviceLocation) -> Vec<Self> {
        let mut capabilities = Vec::new();
        let am = CSpaceAccessMethod::IO;
        let mut cap_ptr = am.read8(loc, PCI_CAP_PTR) as u16;
        while cap_ptr > 0 {
            let cap_vndr = am.read8(loc, cap_ptr);
            let data = match cap_vndr {
                0x01 => CapabilityData::PM(CapabilityPMData::new(loc, cap_ptr)),
                0x02 => CapabilityData::AGP,
                0x03 => CapabilityData::VPD,
                0x04 => CapabilityData::SLOTID,
                0x05 => CapabilityData::MSI(CapabilityMSIData::new(loc, cap_ptr)),
                0x06 => CapabilityData::CHSWP,
                0x07 => CapabilityData::PCIX,
                0x08 => CapabilityData::HP,
                0x09 => CapabilityData::VNDR(CapabilityVNDRData::new(loc, cap_ptr)),
                0x0A => CapabilityData::DBG,
                0x0B => CapabilityData::CCRC,
                0x0C => CapabilityData::SHPC,
                0x0D => CapabilityData::SSVID,
                0x0E => CapabilityData::AGP3,
                0x0F => CapabilityData::SECDEV,
                0x10 => CapabilityData::EXP(CapabilityEXPData::new(loc, cap_ptr)),
                0x11 => CapabilityData::MSIX(CapabilityMSIXData::new(loc, cap_ptr)),
                0x12 => CapabilityData::SATA(CapabilitySATAData::new(loc, cap_ptr)),
                0x13 => CapabilityData::AF,
                0x14 => CapabilityData::EA,
                _ => CapabilityData::Unknown(cap_vndr),
            };
            capabilities.push(Self {
                cap_ptr: cap_ptr,
                data: data,
            });
            cap_ptr = am.read8(loc, cap_ptr + 1) as u16;
        }
        capabilities
    }
}
