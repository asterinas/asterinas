use crate::util::CSpaceAccessMethod;
use jinux_frame::bus::pci::PciDeviceLocation;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct CapabilityEXPData {
    interrupt_message_number: u16,
    slot_implemented: u16,
    device_port_type: u16,
    cap_version: u16,
}

impl CapabilityEXPData {
    pub(crate) fn new(loc: PciDeviceLocation, cap_ptr: u16) -> Self {
        let am = CSpaceAccessMethod::IO;
        let cap = am.read16(loc, cap_ptr + 0x2);
        Self {
            interrupt_message_number: (cap >> 9) & 0b11111,
            slot_implemented: (cap >> 8) & 0x1,
            device_port_type: (cap >> 4) & 0xf,
            cap_version: cap & 0xf,
        }
    }
}
