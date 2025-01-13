// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, collections::linked_list::LinkedList};
use log::debug;
use ostd::sync::SpinLock;
use crate::{
    device::{crypto::config::CryptoFeatures, VirtioDeviceError},
    queue::VirtQueue,
    transport::{ConfigManager, VirtioTransport},
};

use super::config::VirtioCryptoConfig;

pub struct CryptoDevice{
    config_manager: ConfigManager<VirtioCryptoConfig>,
    data_queues: SpinLock<LinkedList<VirtQueue>>,
    control_queue: SpinLock<VirtQueue>,
}

impl CryptoDevice {
    pub fn negotiate_features(features: u64) -> u64 {
        features
    }

    pub fn init(mut transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        let config_manager = VirtioCryptoConfig::new_manager(transport.as_ref());
        debug!("virtio_crypto_config = {:?}", config_manager.read_config());
        transport.finish_init();
        
        drop(transport);
        Ok(())
    }
}