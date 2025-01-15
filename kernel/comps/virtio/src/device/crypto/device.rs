// SPDX-License-Identifier: MPL-2.0


use core::hint::spin_loop;

use alloc::{boxed::Box, fmt::Debug, string::ToString, sync::Arc};
use aster_crypto::{AnyCryptoDevice, CryptoCipherAlgorithm, CryptoError, CryptoHashAlgorithm, CryptoOperation};
use log::{debug, warn};
use ostd::{mm::{DmaDirection, DmaStream, DmaStreamSlice, FrameAllocOptions, VmIo}, sync::SpinLock, trap::TrapFrame, Pod};
use crate::{
    device::{crypto::config::CryptoFeatures, VirtioDeviceError},
    queue::VirtQueue,
    transport::{ConfigManager, VirtioTransport},
};
use crate::device::crypto::session::*;
use crate::device::crypto::service::*;
use super::config::VirtioCryptoConfig;

pub struct CryptoDevice{
    transport: SpinLock<Box<dyn VirtioTransport>>,
    config_manager: ConfigManager<VirtioCryptoConfig>,
    data_queue: SpinLock<VirtQueue>,
    control_queue: SpinLock<VirtQueue>,
    control_buffer: DmaStream,
    data_buffer: DmaStream
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

        debug!("crypto features = {:?}", crypto_features);
        crypto_features.bits()
    }

    pub fn init(mut transport: Box<dyn VirtioTransport>) -> Result<(), VirtioDeviceError> {
        let config_manager = VirtioCryptoConfig::new_manager(transport.as_ref());
        let config = config_manager.read_config();
        debug!("virtio_crypto_config = {:?}", config);

        // let max_queue_num = config.max_dataqueues as u16;
        let max_queue_num = 64;
        let data_queue : SpinLock<VirtQueue>  = 
            SpinLock::new(VirtQueue::new(0, max_queue_num, transport.as_mut()).unwrap());
            

        let control_queue: SpinLock<VirtQueue>  = 
            SpinLock::new(VirtQueue::new(1, max_queue_num, transport.as_mut()).unwrap());


        let control_buffer = {
            let segment = FrameAllocOptions::new().alloc_segment(1).unwrap();
            DmaStream::map(segment.into(), DmaDirection::Bidirectional, false).unwrap()
        };

        let data_buffer = {
            let segment = FrameAllocOptions::new().alloc_segment(1).unwrap();
            DmaStream::map(segment.into(), DmaDirection::Bidirectional, false).unwrap()
        };
        
        let device = Arc::new(Self{
            config_manager,
            control_queue,
            data_queue,
            control_buffer,
            data_buffer,
            transport: SpinLock::new(transport),
        });

        fn config_space_change(_: &TrapFrame) {
            debug!("crypto device config space change");
        }
        
        device
            .transport
            .lock()
            .register_cfg_callback(Box::new(config_space_change))
            .unwrap();

        device.transport.lock().finish_init();

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

impl CryptoDevice {

    pub fn destroy_session(&self, operation: CryptoSessionOperation, session_id: i64) -> Result<u8, CryptoError>{
        let ctrl_slice = DmaStreamSlice::new(&self.control_buffer, 0, 72);
        let ctrl_resp_slice = DmaStreamSlice::new(&self.control_buffer, 72, 88);
        self.control_queue.lock().add_dma_buf(&[&ctrl_slice], &[&ctrl_resp_slice]).unwrap();

        let header = CryptoCtrlHeader {
            opcode : operation as i32,
            algo : 0 as _,
            flag : 0,
            reserved : 0
        };

        let req = CryptoDestroySessionReq {
            header,
            flf : VirtioCryptoDestroySessionFlf {
                session_id : session_id
            }
        };

        debug!("send header: bytes: {:?}, len = {:?}, supp_bits:{:?}", 
                req.as_bytes(), &req.to_bytes(true).len(), self.config_manager.read_config().cipher_algo_l);
        
        ctrl_slice.write_bytes(0, &req.to_bytes(true)).unwrap();

        if self.control_queue.lock().should_notify() {
            self.control_queue.lock().notify();
        }
    
        while ! self.control_queue.lock().can_pop(){
            spin_loop();
        }
    
        self.control_queue.lock().pop_used().unwrap();
        ctrl_resp_slice.sync().unwrap();
    
        let mut reader = ctrl_resp_slice.reader().unwrap();
        let res = reader.read_val::<VirtioCryptoDestroySessionInput>().unwrap();
        
        debug!("receive feedback:{:?}", res);

        res.get_result()
    }

    fn create_session<T: CryptoSessionRequest>(&self, req: T, vlf: &[u8], padding: bool)->Result<i64, CryptoError>{
        let vlf_len: i32 = vlf.len() as _;
        let ctrl_slice = DmaStreamSlice::new(&self.control_buffer, 0, 72);
        let ctrl_resp_slice = DmaStreamSlice::new(&self.control_buffer, (72 + vlf_len) as _, 16);
        
        let ctrl_vlf_slice = if vlf.len() > 0{
            let ctrl_vlf_slice = DmaStreamSlice::new(&self.control_buffer, 72, vlf_len as _);
            self.control_queue.lock().add_dma_buf(&[&ctrl_slice, &ctrl_vlf_slice], &[&ctrl_resp_slice]).unwrap();
            Some(ctrl_vlf_slice)
        }else{
            self.control_queue.lock().add_dma_buf(&[&ctrl_slice], &[&ctrl_resp_slice]).unwrap();
            None
        };

        debug!("send header: bytes: {:?}, len = {:?}", 
                req.as_bytes(), req.as_bytes().len());
        
        ctrl_slice.write_bytes(0, &req.to_bytes(padding)).unwrap();
        if let Some(ctrl_vlf_slice) = ctrl_vlf_slice{
            ctrl_vlf_slice.write_bytes(0, vlf).unwrap();
        }

        if self.control_queue.lock().should_notify() {
            self.control_queue.lock().notify();
        }
    
        while ! self.control_queue.lock().can_pop(){
            spin_loop();
        }
    
        self.control_queue.lock().pop_used().unwrap();
        ctrl_resp_slice.sync().unwrap();
    
        let mut reader = ctrl_resp_slice.reader().unwrap();
        let res = reader.read_val::<VirtioCryptoSessionInput>().unwrap();
        
        debug!("receive feedback:{:?}", res);

        res.get_result()
    }
}

impl AnyCryptoDevice for CryptoDevice{
    fn test_device(&self){
        //
    }

    fn create_hash_session(&self, algo: CryptoHashAlgorithm, result_len: u32)->Result<i64, CryptoError>{
        debug!("[CRYPTO] trying to create hash session");
    
        let header = CryptoCtrlHeader { 
            opcode: CryptoSessionOperation::HashCreate as i32, 
            algo: algo as _,
            flag: 0, 
            reserved: 0
        };
    
        let req = CryptoHashSessionReq{
            header,
            flf: VirtioCryptoHashCreateSessionFlf::new(algo, result_len),
        };
        
        self.create_session(req, &[], true)
    }

    fn create_cipher_session(&self, algo: CryptoCipherAlgorithm, op: CryptoOperation, key: &[u8])->Result<i64, CryptoError>{
        debug!("[CRYPTO] trying to create cipher session");

        let key_len: i32 = key.len() as _;
        let header = CryptoCtrlHeader { 
            opcode: CryptoSessionOperation::CipherCreate as i32, 
            algo: algo as _,
            flag: 0, 
            reserved: 0
        };
    
        let req = CryptoCipherSessionReq::new(header, algo, key_len, op);

        self.create_session(req, key, true)
    }

    fn destroy_cipher_session(&self, session_id: i64) -> Result<u8, CryptoError> {
        self.destroy_session(CryptoSessionOperation::CipherDestroy, session_id)
    
    }
}