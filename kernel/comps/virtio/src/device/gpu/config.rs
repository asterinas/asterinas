use core::mem::offset_of;

use aster_util::safe_ptr::SafePtr;
use ostd::Pod;

use crate::transport::{ConfigManager, VirtioTransport};

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