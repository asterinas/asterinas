// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use super::device::SocketDevice;
use crate::{
    bus::VirtioDevice,
    device::{VirtioDeviceError, VirtioDeviceType},
    driver::VirtioDriver,
};

pub(crate) struct VirtioVsockDriver;

impl VirtioVsockDriver {
    pub(crate) fn new() -> Self {
        super::init();
        Self
    }
}

impl VirtioDriver for VirtioVsockDriver {
    fn name(&self) -> &str {
        "virtio_vsock"
    }

    fn match_data(&self) -> &VirtioDeviceType {
        &VirtioDeviceType::Socket
    }

    fn negotiate_features(&self, offered: u64) -> u64 {
        SocketDevice::negotiate_features(offered)
    }

    fn probe(
        &self,
        device: Arc<VirtioDevice>,
        negotiated_features: u64,
    ) -> Result<(), VirtioDeviceError> {
        SocketDevice::init(device, negotiated_features)
    }
}
