use crate::util::{CSpaceAccessMethod, Location};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct CapabilitySATAData {
    major_revision: u32,
    minor_revision: u32,
    bar_offset: u32,
    bar_location: u32,
}

impl CapabilitySATAData {
    pub(crate) fn new(loc: Location, cap_ptr: u16) -> Self {
        let am = CSpaceAccessMethod::IO;
        let sata_cr0 = am.read32(loc, cap_ptr);
        let sata_cr1 = am.read32(loc, cap_ptr + 0x4);
        Self {
            major_revision: (sata_cr0 >> 20) & 0xf,
            minor_revision: (sata_cr0 >> 16) & 0xf,
            bar_offset: (sata_cr1 >> 4) & 0xfffff,
            bar_location: sata_cr1 & 0xf,
        }
    }
}
