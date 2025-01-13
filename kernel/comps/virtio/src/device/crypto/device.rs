// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, collections::linked_list::LinkedList, string::ToString, sync::Arc, fmt::Debug};
use aster_crypto::AnyCryptoDevice;
use log::debug;
use ostd::sync::SpinLock;
use crate::{
    device::VirtioDeviceError,
    queue::VirtQueue,
    transport::{ConfigManager, VirtioTransport},
};

use super::config::VirtioCryptoConfig;

pub struct CryptoDevice{
    transport: SpinLock<Box<dyn VirtioTransport>>,
    config_manager: ConfigManager<VirtioCryptoConfig>,
    data_queues: SpinLock<LinkedList<VirtQueue>>,
    control_queue: SpinLock<VirtQueue>,
}

impl AnyCryptoDevice for CryptoDevice{

}

impl CryptoDevice {
    pub fn negotiate_features(features: u64) -> u64 {
        features
    }

    pub fn init(mut transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        let config_manager = VirtioCryptoConfig::new_manager(transport.as_ref());
        let config = config_manager.read_config();
        debug!("virtio_crypto_config = {:?}", config);

        let ctrl_queue_idx = (config.max_dataqueues - 1) as u16;
        let data_queues: SpinLock<LinkedList<VirtQueue>> = SpinLock::new(LinkedList::new());

        let control_queue: SpinLock<VirtQueue>  = 
            SpinLock::new(VirtQueue::new(ctrl_queue_idx, 2, transport.as_mut()).unwrap());
        
        let device = Arc::new(Self{
            config_manager,
            control_queue,
            data_queues,
            transport: SpinLock::new(transport)
        });
        
        let mut transport = device.transport.disable_irq().lock();
        transport.finish_init();
        drop(transport);

    aster_crypto::register_device(super::DEVICE_NAME.to_string(), device);

        Ok(())
    }
}

impl Debug for CryptoDevice {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CryptoDevice")
            .field("config", &self.config_manager.read_config())
            .field("transport", &self.transport)
            .field("data_queues", &self.data_queues)
            .field("control_queue", &self.control_queue)
            .finish()
    }
}
