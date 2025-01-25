use core::mem::offset_of;

use aster_util::safe_ptr::SafePtr;
use ostd::Pod;

use crate::transport::{ConfigManager, VirtioTransport};

bitflags::bitflags! {
    pub struct GPUFeatures: u64{
        // 支持 Virgl 3D 模式
        const VIRTIO_GPU_F_VIRGL = 1 << 0;
        // 需要支持 EDID（扩展显示识别数据）
		const VIRTIO_GPU_F_EDID = 1 << 1;
        // GPU 资源支持 UUID 分配，即能够为 GPU 资源（如纹理、缓冲区等）分配唯一的标识符（UUID），
        // 并支持将这些资源导出到其他 Virtio 设备。
		const VIRTIO_GPU_F_RESOURCE_UUID = 1 << 2;
        // 支持基于大小的资源 Blob
		const VIRTIO_GPU_F_RESOURCE_BLOB = 1 << 3;
        // 需要支持上下文初始化
		const VIRTIO_GPU_F_CONTEXT_INIT = 1 << 4;
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
        let mut gpu_config = VirtioGPUConfig::new_uninit();

        gpu_config.events_read = self
            .read_once::<u32>(offset_of!(VirtioGPUConfig, events_read))
            .unwrap();
        gpu_config.events_clear = self
            .read_once::<u32>(offset_of!(VirtioGPUConfig, events_clear))
            .unwrap();
        gpu_config.num_scanouts = self
            .read_once::<u32>(offset_of!(VirtioGPUConfig, num_scanouts))
            .unwrap();
        gpu_config.num_capsets = self
            .read_once::<u32>(offset_of!(VirtioGPUConfig, num_capsets))
            .unwrap();
        gpu_config
    }
}