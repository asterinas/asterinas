// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use super::device::EntropyDevice;
use crate::{
    bus::VirtioDevice,
    device::{VirtioDeviceError, VirtioDeviceType},
    driver::VirtioDriver,
};

pub(crate) struct VirtioRngDriver;

impl VirtioRngDriver {
    pub(crate) fn new() -> Self {
        super::init();
        Self
    }
}

impl VirtioDriver for VirtioRngDriver {
    fn name(&self) -> &str {
        "virtio_rng"
    }

    fn match_data(&self) -> &VirtioDeviceType {
        &VirtioDeviceType::Entropy
    }

    fn negotiate_features(&self, _offered: u64) -> u64 {
        0
    }

    fn probe(
        &self,
        device: Arc<VirtioDevice>,
        negotiated_features: u64,
    ) -> Result<(), VirtioDeviceError> {
        EntropyDevice::init(device, negotiated_features)
    }
}
