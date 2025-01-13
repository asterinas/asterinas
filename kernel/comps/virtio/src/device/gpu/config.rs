use core::mem::offset_of;

use aster_util::safe_ptr::SafePtr;
use ostd::Pod;

use crate::transport::{ConfigManager, VirtioTransport};

bitflags::bitflags! {
    pub struct GPUFeatures: u64{
        /// virgl 3D mode is supported.
        const VIRTIO_GPU_F_VIRGL = 1 << 0;
        /// EDID is supported.
        const VIRTIO_GPU_F_EDID = 1 << 1;
        /// assigning resources UUIDs for export to other virtio devices is supported.
        const VIRTIO_GPU_F_RESOURCE_UUID = 1 << 2;
        /// creating and using size-based blob resources is supported.
        const VIRTIO_GPU_F_RESOURCE_BLOB = 1 << 3;
        /// multiple context types and synchronization timelines supported. Requires VIRTIO_GPU_F_VIRGL.
        const VIRTIO_GPU_F_CONTEXT_INIT = 1 << 4;
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(u16)]
pub enum Event {
    /// According to virtio v1.3
    /// https://docs.oasis-open.org/virtio/virtio/v1.3/csd01/virtio-v1.3-csd01.html#x1-3960007:~:text=value%20is%20zero.-,5.7.4.2%20Events,-VIRTIO_GPU_EVENT_DISPLAY
    ///
    /// Display configuration has changed.
    /// The driver SHOULD use the VIRTIO_GPU_CMD_GET_DISPLAY_INFO command to fetch the information from the device.
    /// In case EDID support is negotiated (VIRTIO_GPU_F_EDID feature flag) the device SHOULD also fetch the updated EDID blobs using the VIRTIO_GPU_CMD_GET_EDID command.
    VirtioGPUEventDisplay = 1 << 0,
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct VirtioGPUConfig {
    /// signals pending events to the driver. The driver MUST NOT write to this field.
    pub events_read: u32,
    /// clears pending events in the device.
    /// Writing a ’1’ into a bit will clear the corresponding bit in events_read, mimicking write-to-clear behavior.
    pub events_clear: u32,
    /// specifies the maximum number of scanouts supported by the device. Minimum value is 1, maximum value is 16.
    pub num_scanouts: u32,
    /// specifies the maximum number of capability sets supported by the device. The minimum value is zero.
    pub num_capsets: u32,
}

impl VirtioGPUConfig {
    pub(super) fn new_manager(transport: &dyn VirtioTransport) -> ConfigManager<Self> {
        let safe_ptr = transport
            .device_config_mem()
            .map(|mem| SafePtr::new(mem, 0));
        let bar_space = transport.device_config_bar();
        ConfigManager::new(safe_ptr, bar_space)
    }
}
impl ConfigManager<VirtioGPUConfig> {
    pub(super) fn read_config(&self) -> VirtioGPUConfig {
        let mut console_config = VirtioGPUConfig::new_uninit();
        console_config.events_read = self
            .read_once::<u32>(offset_of!(VirtioGPUConfig, events_read))
            .unwrap();
        console_config.events_clear = self
            .read_once::<u32>(offset_of!(VirtioGPUConfig, events_clear))
            .unwrap();
        console_config.num_scanouts = self
            .read_once::<u32>(offset_of!(VirtioGPUConfig, num_scanouts))
            .unwrap();
        console_config.num_capsets = self
            .read_once::<u32>(offset_of!(VirtioGPUConfig, num_capsets))
            .unwrap();

        console_config
    }
}
