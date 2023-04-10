use crate::util::{CSpaceAccessMethod, Location};
use bitflags::bitflags;
bitflags! {
    pub struct CapabilityMSIMessageControl: u16 {
        const ADDR64_CAPABLE = 1 << 7;
        const MULTIPLE_MESSAGE_ENABLE_2 = 1 << 4;
        const MULTIPLE_MESSAGE_ENABLE_4 = 2 << 4;
        const MULTIPLE_MESSAGE_ENABLE_8 = 3 << 4;
        const MULTIPLE_MESSAGE_ENABLE_16 = 4 << 4;
        const MULTIPLE_MESSAGE_ENABLE_32 = 5 << 4;
        const MULTIPLE_MESSAGE_CAPABLE_2 = 1 << 1;
        const MULTIPLE_MESSAGE_CAPABLE_4 = 2 << 1;
        const MULTIPLE_MESSAGE_CAPABLE_8 = 3 << 1;
        const MULTIPLE_MESSAGE_CAPABLE_16 = 4 << 1;
        const MULTIPLE_MESSAGE_CAPABLE_32 = 5 << 1;
        const ENABLE = 1 << 0;
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct CapabilityMSIData {
    message_control: CapabilityMSIMessageControl,
    message_address: u64,
    message_data: u16,
}

impl CapabilityMSIData {
    pub(crate) fn new(loc: Location, cap_ptr: u16) -> Self {
        let am = CSpaceAccessMethod::IO;
        let message_control =
            CapabilityMSIMessageControl::from_bits_truncate(am.read16(loc, cap_ptr + 0x02));
        let (addr, data) = if message_control.contains(CapabilityMSIMessageControl::ADDR64_CAPABLE)
        {
            // 64bit
            let lo = am.read32(loc, cap_ptr + 0x04) as u64;
            let hi = am.read32(loc, cap_ptr + 0x08) as u64;
            let data = am.read16(loc, cap_ptr + 0x0C);
            ((hi << 32) | lo, data)
        } else {
            // 32bit
            let addr = am.read32(loc, cap_ptr + 0x04) as u64;
            let data = am.read16(loc, cap_ptr + 0x0C);
            (addr, data)
        };
        Self {
            message_control: message_control,
            message_address: addr,
            message_data: data,
        }
    }
}
