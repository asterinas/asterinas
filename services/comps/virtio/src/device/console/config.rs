// SPDX-License-Identifier: MPL-2.0

use aster_frame::io_mem::IoMem;
use aster_util::safe_ptr::SafePtr;
use pod::Pod;

use crate::transport::VirtioTransport;

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
    pub row: u16,
    pub max_nr_ports: u32,
    pub emerg_wr: u32,
}

impl VirtioConsoleConfig {
    pub(super) fn new(transport: &dyn VirtioTransport) -> SafePtr<Self, IoMem> {
        let memory = transport.device_config_memory();
        SafePtr::new(memory, 0)
    }
}
