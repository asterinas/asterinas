// SPDX-License-Identifier: MPL-2.0

//! Virtio-fs device configuration layout and feature bits.

use core::mem::offset_of;

use aster_util::safe_ptr::SafePtr;
use ostd_pod::FromZeros;

use crate::transport::{ConfigManager, VirtioTransport};

bitflags::bitflags! {
    /// The virtio-fs feature bits supported by the driver.
    pub(super) struct FileSystemFeatures: u64 {
        const NOTIFICATION = 1 << 0;
    }
}

impl FileSystemFeatures {
    /// Returns the virtio-fs feature bits supported by this driver.
    pub(super) fn supported_features() -> Self {
        // TODO: Notification support is not yet implemented.
        Self::empty()
    }
}

/// The virtio-fs device configuration layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct VirtioFsConfig {
    tag: [u8; 36],
    num_request_queues: u32,
}

impl VirtioFsConfig {
    /// Returns the UTF-8 mount tag advertised by the device.
    pub(super) fn parse_tag(&self) -> &str {
        let len = self
            .tag
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(self.tag.len());

        match core::str::from_utf8(&self.tag[..len]) {
            Ok(tag) => tag,
            Err(_) => "<invalid-tag>",
        }
    }

    /// Returns the number of request queues advertised by the device.
    pub(super) fn num_request_queues(&self) -> u32 {
        self.num_request_queues
    }

    /// Creates a config-space manager for the virtio-fs device config.
    pub(super) fn new_manager(transport: &dyn VirtioTransport) -> ConfigManager<Self> {
        let safe_ptr = transport
            .device_config_mem()
            .map(|mem| SafePtr::new(mem, 0));
        let bar_space = transport.device_config_bar();

        ConfigManager::new(safe_ptr, bar_space)
    }
}

impl ConfigManager<VirtioFsConfig> {
    /// Reads the virtio-fs device configuration from config space.
    pub(super) fn read_config(&self) -> VirtioFsConfig {
        let mut config = VirtioFsConfig::new_zeroed();

        for (index, byte) in config.tag.iter_mut().enumerate() {
            *byte = self
                .read_once::<u8>(offset_of!(VirtioFsConfig, tag) + index)
                .unwrap();
        }
        config.num_request_queues = self
            .read_once::<u32>(offset_of!(VirtioFsConfig, num_request_queues))
            .unwrap();

        config
    }
}
