// SPDX-License-Identifier: MPL-2.0


use core::hint::spin_loop;

use alloc::{boxed::Box, fmt::Debug, string::ToString, sync::Arc};
use aster_crypto::AnyCryptoDevice;
use log::{debug, warn};
use ostd::{arch::qemu::exit_qemu, mm::{DmaDirection, DmaStream, DmaStreamSlice, FrameAllocOptions}, sync::SpinLock, trap::TrapFrame, Pod};
use crate::{
    device::{crypto::config::CryptoFeatures, VirtioDeviceError},
    queue::VirtQueue,
    transport::{ConfigManager, VirtioTransport},
};
use crate::device::crypto::session::*;
use super::config::VirtioCryptoConfig;

pub struct CryptoDevice{
    transport: SpinLock<Box<dyn VirtioTransport>>,
    config_manager: ConfigManager<VirtioCryptoConfig>,
    data_queue: SpinLock<VirtQueue>,
    control_queue: SpinLock<VirtQueue>,
    pub control_buffer: DmaStream,
    ctrl_resp_buffer: DmaStream,
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

        debug!("{:?}", crypto_features);
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
            DmaStream::map(segment.into(), DmaDirection::ToDevice, false).unwrap()
        };

        let ctrl_resp_buffer = {
            let segment = FrameAllocOptions::new().alloc_segment(1).unwrap();
            DmaStream::map(segment.into(), DmaDirection::FromDevice, false).unwrap()
        };

        let data_buffer = {
            let segment = FrameAllocOptions::new().alloc_segment(1).unwrap();
            DmaStream::map(segment.into(), DmaDirection::ToDevice, false).unwrap()
        };
        
        let device = Arc::new(Self{
            config_manager,
            control_queue,
            data_queue,
            control_buffer,
            ctrl_resp_buffer,
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
        
        device.activate_control_buffer(&mut device.control_queue.lock());

        device.transport.lock().finish_init();

        aster_crypto::register_device(super::DEVICE_NAME.to_string(), device);

        Ok(())
    }

    fn activate_control_buffer(&self, control_queue: &mut VirtQueue){
        let ctrl_slice = DmaStreamSlice::new(&self.control_buffer, 0, 72);
        let ctrl_resp_slice = DmaStreamSlice::new(&self.ctrl_resp_buffer, 0, 16);
        control_queue.add_dma_buf(&[&ctrl_slice], &[&ctrl_resp_slice]).unwrap();

        // if control_queue.should_notify() {
        //     control_queue.notify();
        // }
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



impl AnyCryptoDevice for CryptoDevice{
    fn test_device(&self){
        //test hash session create
        
        debug!("test begin!");
        let algo = CryptoHashAlgorithm::Sha256;
    
        let header = CryptoCtrlHeader { 
            opcode: CryptoSessionOperation::HashCreate as i32, 
            algo: algo as _,
            flag: 0, 
            reserved: 0
        };
    
        let req = CryptoHashSessionReq{
            header,
            flf: VirtioCryptoHashCreateSessionReq::new(algo, 64),
            padding: [0; 12]
        };
    
        let mut writer = self.control_buffer.writer().unwrap();
        let send_res = writer.write_val(&req);
    
        debug!("send header: bytes: {:?}, len = {:?}, resp: {:?}, supp_bits:{:?}", req.as_bytes(), req.as_bytes().len(), send_res, self.config_manager.read_config().hash_algo);
    
        while ! self.control_queue.lock().can_pop(){
            spin_loop();
        }
    
        self.control_queue.lock().pop_used().unwrap();
        self.ctrl_resp_buffer.sync(0..4).unwrap();
    
        let mut reader = self.ctrl_resp_buffer.reader().unwrap();
        let res = reader.read_val::<VirtioCryptoSessionInput>().unwrap();
    
        debug!("get session result: {:?}", res);
    }
}