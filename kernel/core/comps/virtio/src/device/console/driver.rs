// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use super::device::ConsoleDevice;
use crate::{
    bus::VirtioDevice,
    device::{VirtioDeviceError, VirtioDeviceType},
    driver::VirtioDriver,
};

pub(crate) struct VirtioConsoleDriver;

impl VirtioDriver for VirtioConsoleDriver {
    fn name(&self) -> &str {
        "virtio_console"
    }

    fn match_data(&self) -> &VirtioDeviceType {
        &VirtioDeviceType::Console
    }

    fn negotiate_features(&self, offered: u64) -> u64 {
        ConsoleDevice::negotiate_features(offered)
    }

    fn probe(
        &self,
        device: Arc<VirtioDevice>,
        negotiated_features: u64,
    ) -> Result<(), VirtioDeviceError> {
        ConsoleDevice::init(device, negotiated_features)
    }
}
