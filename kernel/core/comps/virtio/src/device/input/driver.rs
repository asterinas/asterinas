// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use super::device::InputDevice;
use crate::{
    bus::VirtioDevice,
    device::{VirtioDeviceError, VirtioDeviceType},
    driver::VirtioDriver,
};

pub(crate) struct VirtioInputDriver;

impl VirtioDriver for VirtioInputDriver {
    fn name(&self) -> &str {
        "virtio_input"
    }

    fn match_data(&self) -> &VirtioDeviceType {
        &VirtioDeviceType::Input
    }

    fn negotiate_features(&self, offered: u64) -> u64 {
        InputDevice::negotiate_features(offered)
    }

    fn probe(
        &self,
        device: Arc<VirtioDevice>,
        negotiated_features: u64,
    ) -> Result<(), VirtioDeviceError> {
        InputDevice::init(device, negotiated_features)
    }
}
