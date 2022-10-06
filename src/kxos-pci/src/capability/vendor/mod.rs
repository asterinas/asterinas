pub mod virtio;

use virtio::CapabilityVirtioData;

use crate::util::{CSpaceAccessMethod, Location};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CapabilityVNDRData {
    /// Virtio
    VIRTIO(CapabilityVirtioData),
}

impl CapabilityVNDRData {
    pub(crate) fn new(loc: Location, cap_ptr: u16) -> Self {
        let am = CSpaceAccessMethod::IO;
        let vid = am.read16(loc, 0);
        match vid {
            0x1af4 => Self::VIRTIO(CapabilityVirtioData::new(loc, cap_ptr)),
            _ => {
                panic!(
                    "unsupport vendor-specific capability, deivce vendor id:{}",
                    vid
                )
            }
        }
    }
}
