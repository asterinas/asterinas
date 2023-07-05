use crate::util::CSpaceAccessMethod;
use jinux_frame::bus::pci::PciDeviceLocation;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct CapabilityVirtioData {
    pub cfg_type: u8,
    pub bar: u8,
    pub offset: u32,
    pub length: u32,
    pub option: Option<u32>,
}

impl CapabilityVirtioData {
    pub(crate) fn new(loc: PciDeviceLocation, cap_ptr: u16) -> Self {
        let am = CSpaceAccessMethod::IO;
        let cap_len = am.read8(loc, cap_ptr + 2);
        let option = if cap_len > 0x10 {
            Some(am.read32(loc, cap_ptr + 16))
        } else {
            None
        };
        Self {
            cfg_type: am.read8(loc, cap_ptr + 3),
            bar: am.read8(loc, cap_ptr + 4),
            offset: am.read32(loc, cap_ptr + 8),
            length: am.read32(loc, cap_ptr + 12),
            option: option,
        }
    }
}
