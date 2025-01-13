// SPDX-License-Identifier: MPL-2.0

use alloc::{boxed::Box, string::ToString, sync::Arc, fmt::Debug};
use aster_crypto::AnyCryptoDevice;
use log::{debug, warn};
use ostd::sync::SpinLock;
use crate::{
    device::{crypto::config::CryptoFeatures, VirtioDeviceError},
    queue::VirtQueue,
    transport::{ConfigManager, VirtioTransport},
};

use super::config::VirtioCryptoConfig;

pub struct CryptoDevice{
    transport: SpinLock<Box<dyn VirtioTransport>>,
    config_manager: ConfigManager<VirtioCryptoConfig>,
    data_queue: SpinLock<VirtQueue>,
    control_queue: SpinLock<VirtQueue>,
}

impl AnyCryptoDevice for CryptoDevice{

}

impl CryptoDevice {
    pub fn negotiate_features(device_features: u64) -> u64 {
        let device_features = CryptoFeatures::from_bits_truncate(device_features);
        let supported_features = CryptoFeatures::support_features();
        let crypto_features = device_features & supported_features;

        if crypto_features != device_features {
            warn!(
                "Virtio crypto contains unsupported device features: {:?}",
                device_features.difference(supported_features)
            );
        }

        debug!("{:?}", crypto_features);
        crypto_features.bits()
    }

    pub fn init(mut transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        let config_manager = VirtioCryptoConfig::new_manager(transport.as_ref());
        let config = config_manager.read_config();
        debug!("virtio_crypto_config = {:?}", config);

        let max_queue_num = config.max_dataqueues as u16;
        let data_queue : SpinLock<VirtQueue>  = 
            SpinLock::new(VirtQueue::new(0, max_queue_num, transport.as_mut()).unwrap());
            

        let control_queue: SpinLock<VirtQueue>  = 
            SpinLock::new(VirtQueue::new(max_queue_num, 1, transport.as_mut()).unwrap());
        
        let device = Arc::new(Self{
            config_manager,
            control_queue,
            data_queue,
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
            .field("data_queue", &self.data_queue)
            .field("control_queue", &self.control_queue)
            .finish()
    }
}
