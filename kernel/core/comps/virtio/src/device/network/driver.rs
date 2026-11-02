// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use super::device::NetworkDevice;
use crate::{
    bus::VirtioDevice,
    device::{VirtioDeviceError, VirtioDeviceType},
    driver::VirtioDriver,
};

pub(crate) struct VirtioNetDriver;

impl VirtioNetDriver {
    pub(crate) fn new() -> Self {
        super::init();
        Self
    }
}

impl VirtioDriver for VirtioNetDriver {
    fn name(&self) -> &str {
        "virtio_net"
    }

    fn match_data(&self) -> &VirtioDeviceType {
        &VirtioDeviceType::Network
    }

    fn negotiate_features(&self, offered: u64) -> u64 {
        NetworkDevice::negotiate_features(offered)
    }

    fn probe(
        &self,
        device: Arc<VirtioDevice>,
        negotiated_features: u64,
    ) -> Result<(), VirtioDeviceError> {
        NetworkDevice::init(device, negotiated_features)
    }
}
