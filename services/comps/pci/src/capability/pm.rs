use crate::util::{CSpaceAccessMethod, Location};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct CapabilityPMData {
    pub pme_support: u32,
    pub d2_support: u32,
    pub d1_support: u32,
    pub aux_current: u32,
    pub dsi: u32,
    pub pme_clock: u32,
    pub version: u32,
}

impl CapabilityPMData {
    pub(crate) fn new(loc: Location, cap_ptr: u16) -> Self {
        let am = CSpaceAccessMethod::IO;
        let cap = am.read32(loc, cap_ptr + 0x4);
        Self {
            pme_support: cap >> 27,
            d2_support: (cap >> 26) & 0x1,
            d1_support: (cap >> 25) & 0x1,
            aux_current: (cap >> 22) & 0x7,
            dsi: (cap >> 21) & 0x1,
            pme_clock: (cap >> 19) & 0x1,
            version: (cap >> 16) & 0x7,
        }
    }
}
