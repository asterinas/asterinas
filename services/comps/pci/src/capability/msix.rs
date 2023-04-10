use crate::util::*;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
#[repr(C)]
pub struct CapabilityMSIXData {
    pub message_control: u16,
    pub table_info: u32,
    pub pba_info: u32,
}

impl CapabilityMSIXData {
    pub fn new(loc: Location, cap_ptr: u16) -> Self {
        let am = CSpaceAccessMethod::IO;
        Self {
            message_control: am.read16(loc, cap_ptr + 2),
            table_info: am.read32(loc, cap_ptr + 4),
            pba_info: am.read32(loc, cap_ptr + 8),
        }
    }
}
