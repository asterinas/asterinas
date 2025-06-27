// SPDX-License-Identifier: MPL-2.0

use core::mem::offset_of;

use ostd::Pod;
use aster_util::safe_ptr::SafePtr;
use crate::transport::{ConfigManager, VirtioTransport};

bitflags::bitflags! {
    pub struct CryptoFeatures: u64{
        /// Revision 1 has a specific request format and other enhancements (which result in some additional requirements).
        const VIRTIO_CRYPTO_F_REVISION_1            = 1 << 0;
        /// stateless mode requests are supported by the CIPHER service.
        const VIRTIO_CRYPTO_F_CIPHER_STATELESS_MODE = 1 << 1;
        /// stateless mode requests are supported by the HASH service.
        const VIRTIO_CRYPTO_F_HASH_STATELESS_MODE   = 1 << 2;
        /// stateless mode requests are supported by the MAC service.
        const VIRTIO_CRYPTO_F_MAC_STATELESS_MODE    = 1 << 3;
        /// stateless mode requests are supported by the AEAD service.
        const VIRTIO_CRYPTO_F_AEAD_STATELESS_MODE   = 1 << 4;
    }
}

impl CryptoFeatures {
    pub fn support_features() -> Self {
        Self::empty()
    }
}

bitflags::bitflags! {
    #[repr(C)]
    #[derive(Pod)]
    pub struct CryptoService: u32 {
        /// CIPHER service
        const VIRTIO_CRYPTO_SERVICE_CIPHER = 1 << 0;
        /// HASH service
        const VIRTIO_CRYPTO_SERVICE_HASH = 1 << 1;
        /// MAC (Message Authentication Codes) service
        const VIRTIO_CRYPTO_SERVICE_MAC = 1 << 2;
        /// AEAD (Authenticated Encryption with Associated Data) service
        const VIRTIO_CRYPTO_SERVICE_AEAD = 1 << 3;
        /// ???
        const VIRTIO_CRYPTO_SERVICE_AKCIPHER = 1 << 4;
    }
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct VirtioCryptoConfig {
    pub status: u32,
    pub max_dataqueues: u32,
    pub crypto_services: CryptoService,

    /* Detailed algorithms mask */ 
    pub cipher_algo_l: u32,
    pub cipher_algo_h: u32,
    pub hash_algo: u32,
    pub mac_algo_l: u32,
    pub mac_algo_h: u32,
    pub aead_algo: u32,

    /* Maximum length of cipher key in bytes */ 
    pub max_cipher_key_len: u32,

    /* Maximum length of authenticated key in bytes */ 
    pub max_auth_key_len: u32,
    pub akcipher_algo: u32,
    /* Maximum size of each crypto request’s content in bytes */ 
    pub max_size: u64,
}

impl VirtioCryptoConfig {
    pub(super) fn new_manager(transport: &dyn VirtioTransport) -> ConfigManager<Self> {
        let safe_ptr = transport
            .device_config_mem()
            .map(|mem| SafePtr::new(mem, 0));
        let bar_space = transport.device_config_bar();
        ConfigManager::new(safe_ptr, bar_space)
    }
}

impl ConfigManager<VirtioCryptoConfig> {
    pub(super) fn read_config(&self) -> VirtioCryptoConfig {
        let mut crypto_config = VirtioCryptoConfig::new_uninit();

        crypto_config.status = self
            .read_once::<u32>(offset_of!(VirtioCryptoConfig, status))
            .unwrap();
        crypto_config.max_dataqueues = self
            .read_once::<u32>(offset_of!(VirtioCryptoConfig, max_dataqueues))
            .unwrap();
        crypto_config.crypto_services.bits = self
            .read_once::<u32>(offset_of!(VirtioCryptoConfig, crypto_services))
            .unwrap();

        if crypto_config.crypto_services.contains(CryptoService::VIRTIO_CRYPTO_SERVICE_CIPHER){
            crypto_config.cipher_algo_l = self
                .read_once::<u32>(offset_of!(VirtioCryptoConfig, cipher_algo_l))
                .unwrap();
            crypto_config.cipher_algo_h = self
                .read_once::<u32>(offset_of!(VirtioCryptoConfig, cipher_algo_h))
                .unwrap();
        }else{
            crypto_config.cipher_algo_h = 0;
            crypto_config.cipher_algo_l = 1;
        }

        if crypto_config.crypto_services.contains(CryptoService::VIRTIO_CRYPTO_SERVICE_HASH){
            crypto_config.hash_algo = self
                .read_once::<u32>(offset_of!(VirtioCryptoConfig, hash_algo))
                .unwrap();
        }else{
            crypto_config.hash_algo = 1;
        }
            
        if crypto_config.crypto_services.contains(CryptoService::VIRTIO_CRYPTO_SERVICE_MAC){
            crypto_config.mac_algo_l = self
                .read_once::<u32>(offset_of!(VirtioCryptoConfig, mac_algo_l))
                .unwrap();
            crypto_config.mac_algo_h = self
                .read_once::<u32>(offset_of!(VirtioCryptoConfig, mac_algo_h))
                .unwrap();
        }else{
            crypto_config.mac_algo_l = 1;
            crypto_config.mac_algo_h = 0;
        }

        if crypto_config.crypto_services.contains(CryptoService::VIRTIO_CRYPTO_SERVICE_AEAD){
            crypto_config.aead_algo = self
                .read_once::<u32>(offset_of!(VirtioCryptoConfig, aead_algo))
                .unwrap();
        }else{
            crypto_config.aead_algo = 1;
        }
        
        crypto_config.max_cipher_key_len = self
            .read_once::<u32>(offset_of!(VirtioCryptoConfig, max_cipher_key_len))
            .unwrap();
        crypto_config.max_auth_key_len = self
            .read_once::<u32>(offset_of!(VirtioCryptoConfig, max_auth_key_len))
            .unwrap();
        crypto_config.akcipher_algo = self
            .read_once::<u32>(offset_of!(VirtioCryptoConfig, akcipher_algo))
            .unwrap();

        let max_size_low = self
            .read_once::<u32>(offset_of!(VirtioCryptoConfig, max_size))
            .unwrap() as u64;
        let max_size_high = self
            .read_once::<u32>(offset_of!(VirtioCryptoConfig, max_size) + 4)
            .unwrap() as u64;

        crypto_config.max_size = (max_size_high << 32) | max_size_low;

        crypto_config
    }
}
