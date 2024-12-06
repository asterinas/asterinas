use core::mem::offset_of;

use aster_util::{read_union_fields, safe_ptr::SafePtr};
use ostd::Pod;

use crate::transport::{ConfigManager, VirtioTransport};
use crate::bitflags;


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

bitflags! {
    #[repr(C)]
    #[derive(Pod)]
    pub struct Status: u16 {
        const VIRTIO_GPU_EVENT_DISPLAY  = 1 << 0;
    }
}


#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct VirtioGPUConfig {
    pub events_read: u32,
    pub events_clear: u32,
    pub num_scanouts: u32,
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
