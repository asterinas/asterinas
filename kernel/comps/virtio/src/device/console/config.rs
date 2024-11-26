// SPDX-License-Identifier: MPL-2.0

use core::mem::offset_of;

use aster_util::safe_ptr::SafePtr;
use ostd::Pod;

use crate::transport::{ConfigManager, VirtioTransport};

bitflags::bitflags! {
    pub struct ConsoleFeatures: u64{
        /// Configuration cols and rows are valid.
        const VIRTIO_CONSOLE_F_SIZE = 1 << 0;
        /// Device has support for multiple ports;
        /// max_nr_ports is valid and control virtqueues will be used.
        const VIRTIO_CONSOLE_F_MULTIPORT = 1 << 1;
        /// Device has support for emergency write.
        /// Configuration field emerg_wr is valid.
        const VIRTIO_CONSOLE_F_EMERG_WRITE = 1 << 2;
    }
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct VirtioConsoleConfig {
    pub cols: u16,
    pub rows: u16,
    pub max_nr_ports: u32,
    pub emerg_wr: u32,
}

impl VirtioConsoleConfig {
    pub(super) fn new_manager(transport: &dyn VirtioTransport) -> ConfigManager<Self> {
        let safe_ptr = transport
            .device_config_mem()
            .map(|mem| SafePtr::new(mem, 0));
        let bar_space = transport.device_config_bar();
        ConfigManager::new(safe_ptr, bar_space)
    }
}

impl ConfigManager<VirtioConsoleConfig> {
    pub(super) fn read_config(&self) -> VirtioConsoleConfig {
        let mut console_config = VirtioConsoleConfig::new_uninit();
        // Only following fields are defined in legacy interface.
        console_config.cols = self
            .read_once::<u16>(offset_of!(VirtioConsoleConfig, cols))
            .unwrap();
        console_config.rows = self
            .read_once::<u16>(offset_of!(VirtioConsoleConfig, rows))
            .unwrap();
        console_config.max_nr_ports = self
            .read_once::<u32>(offset_of!(VirtioConsoleConfig, max_nr_ports))
            .unwrap();

        console_config
    }

    /// Performs an emergency write.
    ///
    /// According to the VirtIO spec 5.3.4:
    ///
    /// If `VIRTIO_CONSOLE_F_EMERG_WRITE` is supported then the driver can
    /// use emergency write to output a single character without initializing
    /// virtio queues, or even acknowledging the feature.
    pub(super) fn emerg_write(&self, value: u32) {
        if self.is_modern() {
            self.write_once(offset_of!(VirtioConsoleConfig, emerg_wr), value)
                .unwrap();
        }
    }
}
