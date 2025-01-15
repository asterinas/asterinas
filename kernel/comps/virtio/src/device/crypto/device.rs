// SPDX-License-Identifier: MPL-2.0


use core::{hash, hint::spin_loop};

use alloc::{boxed::Box, fmt::Debug, string::ToString, sync::Arc, vec, vec::Vec};
use aster_crypto::{AnyCryptoDevice, CryptoCurve, CryptoAkCipherAlgorithm, CryptoAkCipherKeyType, CryptoCipherAlgorithm, CryptoError, CryptoHashAlgorithm, CryptoMacAlgorithm, CryptoSymAlgChainOrder, CryptoSymHashMode, CryptoOperation, CryptoSymOp, CryptoHashAlgo, CryptoPaddingAlgo};
use log::{debug, warn};
use ostd::{mm::{DmaDirection, DmaStream, DmaStreamSlice, FrameAllocOptions, VmIo}, sync::SpinLock, trap::TrapFrame, Pod};
use crate::{
    device::{crypto::config::CryptoFeatures, VirtioDeviceError},
    queue::VirtQueue,
    transport::{ConfigManager, VirtioTransport},
};
use crate::device::crypto::session::*;
use crate::device::crypto::service::*;
use super::{config::VirtioCryptoConfig, session};

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

    fn handle_service<T: CryptoServiceRequest>(&self, req: T, vlf: &[u8], rst_len: i32, padding: bool)->Result<Vec<u8>, CryptoError> {
        let vlf_len = vlf.len() as i32;
        let service_slice = DmaStreamSlice::new(&self.data_buffer, 0, 72);
        let service_resp_slice = DmaStreamSlice::new(&self.data_buffer, (72 + vlf_len + rst_len) as _, 2);
        let service_vlf_slice = DmaStreamSlice::new(&self.data_buffer, 72, vlf_len as _);
        let service_rst_slice = DmaStreamSlice::new(&self.data_buffer, (72 + vlf_len) as _, rst_len as _);
        self.data_queue.lock().add_dma_buf(&[&service_slice, &service_vlf_slice], &[&service_rst_slice, &service_resp_slice]).unwrap();

        debug!("send header: bytes: {:?}, len = {:?}", 
                req.as_bytes(), req.as_bytes().len());
        
        service_slice.write_val(0, &req).unwrap();
        service_vlf_slice.write_bytes(0, vlf).unwrap();

        if self.data_queue.lock().should_notify() {
            self.data_queue.lock().notify();
        }
    
        while ! self.data_queue.lock().can_pop(){
            spin_loop();
        }
    
        self.data_queue.lock().pop_used().unwrap();
        service_resp_slice.sync().unwrap();
    
        let mut reader = service_resp_slice.reader().unwrap();
        let status = reader.read_val::<VirtioCryptoInhdr>().unwrap();

        if let Err(err) = status.get_result() {
            return Err(err);
        }
        let mut res: Vec<u8> = vec![0; rst_len as _];
        service_rst_slice.read_bytes(0, &mut res[..]).unwrap();
        debug!("receive feedback:{:?}", res);
        Ok(res)
    }
}

impl AnyCryptoDevice for CryptoDevice{
    fn test_device(&self){
        let res1 = self.create_hash_session(CryptoHashAlgorithm::Sha256, 64);
        debug!("try to create hash session:{:?}", res1);
        let res2 = 
            self.create_cipher_session(CryptoCipherAlgorithm::AesEcb, 
                                        CryptoOperation::Encrypt, &[1; 16]);
        debug!("try to create cipher session:{:?}", res2);

        let id2 = &res2.unwrap();
        let res3=self.handle_cipher_service_req(true, CryptoCipherAlgorithm::AesEcb, id2.clone(), &[0; 16], &[2; 16], 16);
        debug!("try to call cipher service:{:?}", res3);

        // let res4 = self.handle
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
            flf: VirtioCryptoHashSessionFlf::new(algo, result_len),
        };
        
        self.create_session(req, &[], true)
    }

    fn create_mac_session(&self, algo: aster_crypto::CryptoMacAlgorithm, result_len: u32, auth_key: &[u8])->Result<i64, CryptoError> {
        debug!("[CRYPTO] trying to create mac session");

        let key_len: u32 = auth_key.len() as _;
        let header = CryptoCtrlHeader{
            opcode: CryptoSessionOperation::MacCreate as i32, 
            algo: algo as _,
            flag: 0, 
            reserved: 0            
        };

        let req = CryptoMacSessionReq{
            header, flf: VirtioCryptoMacSessionFlf::new(algo, result_len, key_len)
        };

        self.create_session(req, auth_key, true)
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
    
        let flf = VirtioCryptoCipherSessionFlf::new(algo, key_len, op);
        let req = CryptoCipherSessionReq::new(header, VirtioCryptoSymCreateSessionFlf{CipherFlf: flf}, CryptoSymOp::Cipher as _);

        self.create_session(req, key, true)
    }

    fn create_alg_chain_auth_session(&self, algo: CryptoCipherAlgorithm, op: CryptoOperation, alg_chain_order: CryptoSymAlgChainOrder, mac_algo: CryptoMacAlgorithm, result_len: u32, aad_len: i32, cipher_key: &[u8], auth_key: &[u8])->Result<i64, CryptoError> {
        debug!("[CRYPTO] trying to create alg chain auth session");
        let hash_mode = CryptoSymHashMode::Auth;
        let header = CryptoCtrlHeader { 
            opcode: CryptoSessionOperation::CipherCreate as i32, 
            algo: algo as _,
            flag: 0, 
            reserved: 0
        };
        let key_len: u32 = cipher_key.len() as _;

        let cipher_flf = VirtioCryptoCipherSessionFlf::new(algo, key_len as _, op);
        let auth_key_len: u32 = auth_key.len() as _;
        let mac_flf = VirtioCryptoMacSessionFlf::new(mac_algo, result_len, auth_key_len);
        let flf = VirtioCryptoAlgChainSessionFlf::new(alg_chain_order, hash_mode, cipher_flf, VirtioCryptoAlgChainSessionAlgo{mac_flf}, aad_len);
        let req = CryptoCipherSessionReq::new(
            header, 
            VirtioCryptoSymCreateSessionFlf{AlgChainFlf: flf}, 
            CryptoSymOp::AlgorithmChaining
        );

        self.create_session(req, &[cipher_key, auth_key].concat(), true)
    }

    fn create_alg_chain_plain_session(&self, algo: CryptoCipherAlgorithm, op: CryptoOperation, alg_chain_order: CryptoSymAlgChainOrder, hash_algo: CryptoHashAlgorithm, result_len: u32, aad_len: i32, cipher_key: &[u8])->Result<i64, CryptoError> {
        debug!("[CRYPTO] trying to create alg chain plain session");
        let hash_mode = CryptoSymHashMode::Plain;
        let header = CryptoCtrlHeader { 
            opcode: CryptoSessionOperation::CipherCreate as i32, 
            algo: algo as _,
            flag: 0, 
            reserved: 0
        };
        let key_len: u32 = cipher_key.len() as _;
        let cipher_flf = VirtioCryptoCipherSessionFlf::new(algo, key_len as _, op);
        let hash_flf = VirtioCryptoHashSessionFlf::new(hash_algo, result_len);
        let flf = VirtioCryptoAlgChainSessionFlf::new(alg_chain_order, hash_mode, cipher_flf, VirtioCryptoAlgChainSessionAlgo{hash_flf}, aad_len);
        let req = CryptoCipherSessionReq::new(
            header, 
            VirtioCryptoSymCreateSessionFlf{AlgChainFlf: flf}, 
            CryptoSymOp::AlgorithmChaining
        );

        self.create_session(req, cipher_key, true)
    }

    fn handle_cipher_service_req(&self, encrypt : bool, algo: CryptoCipherAlgorithm, session_id : i64, iv : &[u8], src_data : &[u8], dst_data_len : i32) -> Result<Vec<u8>, CryptoError> {

        debug!("[CRYPTO] trying to handle cipher service request");
        let header = CryptoServiceHeader {
            opcode : if encrypt {CryptoServiceOperation::CipherEncrypt} else  {CryptoServiceOperation::CipherDecrypt} as _,
            algo : algo as _,
            session_id,
            flag : 1, // VIRTIO_CRYPTO_FLAG_SESSION_MODE
            padding : 0
        };
        let src_data_len = src_data.len() as i32;
        let iv_len = iv.len() as i32;
        let flf = VirtioCryptoCipherDataFlf::new(iv_len, src_data_len, dst_data_len);
        let req = CryptoCipherServiceReq {
            header,
            op_flf : VirtioCryptoSymDataFlf {
                op_type_flf : VirtioCryptoSymDataFlfWrapper{ CipherFlf : flf},
                op_type : CryptoSymOp::Cipher as _,
                padding : 0
            }
        };

        let vlf = &[iv, src_data].concat();

        let dst_data = self.handle_service(req, vlf, dst_data_len, true);
        dst_data

    }

    fn handle_alg_chain_service_req(&self, encrypt : bool, algo: CryptoCipherAlgorithm, session_id: i64, iv : &[u8], src_data : &[u8], dst_data_len: i32, cipher_start_src_offset: i32, len_to_cipher: i32, hash_start_src_offset: i32, len_to_hash: i32, aad_len: i32, hash_result_len: i32) -> Result<(Vec<u8>, Vec<u8>), CryptoError> {
        debug!("[CRYPTO] trying to handle cipher service request");
        let header = CryptoServiceHeader {
            opcode : if encrypt {CryptoServiceOperation::CipherEncrypt} else  {CryptoServiceOperation::CipherDecrypt} as _,
            algo : algo as _,
            session_id,
            flag : 1, // VIRTIO_CRYPTO_FLAG_SESSION_MODE
            padding : 0
        };
        let src_data_len = src_data.len() as i32;
        let iv_len = iv.len() as i32;
        let flf = VirtioCryptoAlgChainDataFlf::new(
            iv_len, 
            src_data_len, 
            dst_data_len, 
            cipher_start_src_offset, 
            len_to_cipher, 
            hash_start_src_offset, 
            len_to_hash, 
            aad_len, 
            hash_result_len
        );
        let req = CryptoCipherServiceReq {
            header,
            op_flf : VirtioCryptoSymDataFlf {
                op_type_flf : VirtioCryptoSymDataFlfWrapper{ AlgChainFlf : flf},
                op_type : CryptoSymOp::AlgorithmChaining as _,
                padding : 0
            }
        };

        let vlf = &[iv, src_data].concat();

        let dst_data = self.handle_service(req, vlf, dst_data_len + hash_result_len, true);
        match dst_data {
            Ok(data) => {
                let (fi, sc) = data.split_at(dst_data_len as _);
                Ok((fi.to_vec(), sc.to_vec()))
            }
            Err(err) => Err(err)
        }
    }

    fn destroy_cipher_session(&self, session_id: i64) -> Result<u8, CryptoError> {
        self.destroy_session(CryptoSessionOperation::CipherDestroy, session_id)
    
    }

    fn create_akcipher_rsa_session(&self, algo: CryptoAkCipherAlgorithm,
                                   op: CryptoOperation,
                                   padding_algo: CryptoPaddingAlgo,
                                   hash_algo: CryptoHashAlgo,
                                   key_type: CryptoAkCipherKeyType,
                                   key: &[u8],
    ) -> Result<i64, CryptoError> {
        debug!("[CRYPTO] trying to create akcipher rsa session");

        let header = CryptoCtrlHeader {
            opcode: CryptoSessionOperation::AkCipherCreate as i32,
            algo: algo as _,
            flag: 0,
            reserved: 0,
        };

        let key_len : u32 = key.len() as _;
        let para = VirtioCryptoRSAPara {
            padding_algo: padding_algo as _,
            hash_algo: hash_algo as _,
        };
        let algo_flf = VirtioCryptoAlgoFif { rsa: para };
        let flf = VirtioCryptoAkCipherSessionFlf::new(algo, key_type, key_len, algo_flf);
        let req = CryptoAkCipherSessionReq {
            header,
            flf,
        };

        self.create_session(req, &key, true)
    }

    fn create_akcipher_ecdsa_session(&self, algo: CryptoAkCipherAlgorithm,
                                     op: CryptoOperation,
                                     curve_id: CryptoCurve,
                                     key_type: CryptoAkCipherKeyType,
                                     key: &[u8],
    ) -> Result<i64, CryptoError> {
        debug!("[CRYPTO] trying to create akcipher ecdsa session");

        let header = CryptoCtrlHeader {
            opcode: CryptoSessionOperation::AkCipherCreate as i32,
            algo: algo as _,
            flag: 0,
            reserved: 0,
        };

        let key_len : u32 = key.len() as _;
        let para = VirtioCryptoECDSAPara {
            curve_id: curve_id as _,
        };
        let algo_flf = VirtioCryptoAlgoFif { ecdsa: para };
        let flf = VirtioCryptoAkCipherSessionFlf::new(algo, key_type, key_len, algo_flf);
        let req = CryptoAkCipherSessionReq {
            header,
            flf,
        };

        self.create_session(req, &key, true)
    }

    fn handle_akcipher_serivce_req(&self, encrypt : bool, algo: CryptoAkCipherAlgorithm, session_id: i64, src_data : &[u8], dst_data_len : i32) -> Result<Vec<u8>, CryptoError> {
        debug!("[CRYPTO] trying to handle akcipher service request");
        let header = CryptoServiceHeader {
            opcode : if encrypt {CryptoServiceOperation::CipherEncrypt} else  {CryptoServiceOperation::CipherDecrypt} as _,
            algo : algo as _,
            session_id,
            flag : 1, // VIRTIO_CRYPTO_FLAG_SESSION_MODE
            padding : 0
        };
        let src_data_len = src_data.len() as i32;
        let flf = VirtioCryptoAkcipherDataFlf::new(src_data_len, dst_data_len);
        let req = CryptoAkCipherServiceReq {
            header,
            flf
        };

        let vlf = src_data;

        let dst_data = self.handle_service(req, vlf, dst_data_len, true);
        dst_data
    }

    fn destroy_akcipher_session(&self, session_id: i64) -> Result<u8, CryptoError> {
        self.destroy_session(CryptoSessionOperation::AkCipherDestroy, session_id)
    }
}