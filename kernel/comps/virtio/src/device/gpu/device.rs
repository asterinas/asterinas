use alloc::boxed::Box;
use log::debug;
use ostd::sync::SpinLock;
use crate::{device::VirtioDeviceError, queue::VirtQueue, transport::{ConfigManager, VirtioTransport}};

use super::config::VirtioGPUConfig;

pub struct GPUDevice {
    config_manager: ConfigManager<VirtioGPUConfig>,
    transport: SpinLock<Box<dyn VirtioTransport>>,
    receive_queue: SpinLock<VirtQueue>,
    transmit_queue: SpinLock<VirtQueue>,
}

impl GPUDevice {
    pub fn negotiate_features(features: u64) -> u64 {
        features
    }

    pub fn init(mut transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        
        let config_manager = VirtioGPUConfig::new_manager(transport.as_ref());
        use ostd::early_println;
        early_println!("[INFO] GPU Config = {:?}", config_manager.read_config());
        const REQUEST_QUEUE_INDEX: u16 = 0;
        Ok(())
    }
}

