// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use super::device::BlockDevice;
use crate::{
    bus::VirtioDevice,
    device::{VirtioDeviceError, VirtioDeviceType},
    driver::VirtioDriver,
};

pub(crate) struct VirtioBlockDriver;

impl VirtioDriver for VirtioBlockDriver {
    fn name(&self) -> &str {
        "virtio_blk"
    }

    fn match_data(&self) -> &VirtioDeviceType {
        &VirtioDeviceType::Block
    }

    fn negotiate_features(&self, offered: u64) -> u64 {
        BlockDevice::negotiate_features(offered)
    }

    fn probe(
        &self,
        device: Arc<VirtioDevice>,
        negotiated_features: u64,
    ) -> Result<(), VirtioDeviceError> {
        BlockDevice::init(device, negotiated_features)
    }
}
