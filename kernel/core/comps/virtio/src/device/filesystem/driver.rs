// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use super::device::FileSystemDevice;
use crate::{
    bus::VirtioDevice,
    device::{VirtioDeviceError, VirtioDeviceType},
    driver::VirtioDriver,
};

pub(crate) struct VirtioFsDriver;

impl VirtioDriver for VirtioFsDriver {
    fn name(&self) -> &str {
        "virtio_fs"
    }

    fn match_data(&self) -> &VirtioDeviceType {
        &VirtioDeviceType::FileSystem
    }

    fn negotiate_features(&self, offered: u64) -> u64 {
        FileSystemDevice::negotiate_features(offered)
    }

    fn probe(
        &self,
        device: Arc<VirtioDevice>,
        negotiated_features: u64,
    ) -> Result<(), VirtioDeviceError> {
        FileSystemDevice::init(device, negotiated_features)
    }
}
