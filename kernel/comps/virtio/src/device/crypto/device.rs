// SPDX-License-Identifier: MPL-2.0


use core::hint::spin_loop;

use alloc::{boxed::Box, fmt::Debug, string::ToString, sync::Arc, vec, vec::Vec};
use aster_crypto::*;
use aster_input::key;
use log::{debug, warn};
use ostd::{mm::{DmaDirection, DmaStream, DmaStreamSlice, FrameAllocOptions, VmIo}, sync::SpinLock, trap::TrapFrame};
use crate::{
    device::{crypto::config::CryptoFeatures, VirtioDeviceError},
    queue::VirtQueue,
    transport::{ConfigManager, VirtioTransport},
};
use crate::device::crypto::header::*;
use super::config::VirtioCryptoConfig;

pub struct CryptoDevice{
    transport: SpinLock<Box<dyn VirtioTransport>>,
    config_manager: ConfigManager<VirtioCryptoConfig>,
    control_queue: SpinLock<VirtQueue>,
    data_queue: SpinLock<VirtQueue>,
    control_buffer: DmaStream,
    data_buffer: DmaStream,
    revision_1: bool
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
        let features = CryptoFeatures::from_bits_truncate(Self::negotiate_features(
            transport.read_device_features(),
        ));
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
            revision_1: features.contains(CryptoFeatures::VIRTIO_CRYPTO_F_REVISION_1)
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
    fn create_session<T: CtrlFlfPadding>(&self, req: CryptoSessionRequest<T>, vlf: &[u8])->Result<i64, CryptoError>{
        let vlf_len: i32 = vlf.len() as _;
        let req_len: i32 = if self.revision_1 {req.len() as _} else {72};
        let revision_1 = self.revision_1;

        let ctrl_slice = DmaStreamSlice::new(&self.control_buffer, 0, req_len as _);
        let ctrl_resp_slice = DmaStreamSlice::new(&self.control_buffer, (req_len + vlf_len) as _, 16);
        
        let ctrl_vlf_slice = if vlf.len() > 0{
            let ctrl_vlf_slice = DmaStreamSlice::new(&self.control_buffer, req_len as _, vlf_len as _);
            self.control_queue.lock().add_dma_buf(&[&ctrl_slice, &ctrl_vlf_slice], &[&ctrl_resp_slice]).unwrap();
            Some(ctrl_vlf_slice)
        }else{
            self.control_queue.lock().add_dma_buf(&[&ctrl_slice], &[&ctrl_resp_slice]).unwrap();
            None
        };

        debug!("send header: bytes: {:?}, len = {:?}", 
                &req.to_bytes(revision_1), req.to_bytes(revision_1).len());
        
        ctrl_slice.write_bytes(0, &req.to_bytes(revision_1)).unwrap();
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
        let res = reader.read_val::<CryptoSessionInput>().unwrap();
        
        debug!("receive feedback:{:?}", res);

        res.get_result()
    }

    pub fn destroy_session(&self, operation: CryptoSessionOperation, session_id: i64) -> Result<(), CryptoError>{

        let revision_1 = self.revision_1;

        let header = CryptoCtrlHeader {
            opcode : operation as i32,
            algo : 0 as _,
            flag : 0,
            reserved : 0
        };

        let req = CryptoSessionRequest {
            header,
            flf : CryptoDestroySessionFlf{ session_id }
        };

        let req_len: i32 = if self.revision_1 {req.len() as _} else {72};

        let ctrl_slice = DmaStreamSlice::new(&self.control_buffer, 0, req_len as _);
        let ctrl_resp_slice = DmaStreamSlice::new(&self.control_buffer, req_len as _, 16);
        self.control_queue.lock().add_dma_buf(&[&ctrl_slice], &[&ctrl_resp_slice]).unwrap();

        debug!("send header: bytes: {:?}, len = {:?}, supp_bits:{:?}", 
            &req.to_bytes(revision_1), &req.to_bytes(revision_1).len(), self.config_manager.read_config().cipher_algo_l);
        
        ctrl_slice.write_bytes(0, &req.to_bytes(revision_1)).unwrap();

        if self.control_queue.lock().should_notify() {
            self.control_queue.lock().notify();
        }
    
        while ! self.control_queue.lock().can_pop(){
            spin_loop();
        }
    
        self.control_queue.lock().pop_used().unwrap();
        ctrl_resp_slice.sync().unwrap();
    
        let mut reader = ctrl_resp_slice.reader().unwrap();
        let res = reader.read_val::<CryptoDestroySessionInput>().unwrap();
        
        debug!("receive feedback:{:?}", res);

        match res.get_result() {
            Ok(status) => Ok(()),
            Err(err) => Err(err)
        }
    }

    fn handle_service<T: DataFlfPadding>(&self, req: CryptoServiceRequest<T>, vlf: &[u8], rst_len: i32)->Result<Vec<u8>, CryptoError> {
        let revision_1 = self.revision_1;
        let req_len: i32 = if revision_1 {req.len() as _} else {72};
        let vlf_len: i32 = vlf.len() as _;
        let service_slice = DmaStreamSlice::new(&self.data_buffer, 0, req_len as _);
        let service_resp_slice = DmaStreamSlice::new(&self.data_buffer, (req_len + vlf_len + rst_len) as _, 2);
        let service_vlf_slice = DmaStreamSlice::new(&self.data_buffer, req_len as _, vlf_len as _);
        let service_rst_slice = DmaStreamSlice::new(&self.data_buffer, (req_len + vlf_len) as _, rst_len as _);
        self.data_queue.lock().add_dma_buf(&[&service_slice, &service_vlf_slice], &[&service_rst_slice, &service_resp_slice]).unwrap();

        debug!("send header: bytes: {:?}, len = {:?}", 
                req.to_bytes(revision_1), req.to_bytes(revision_1).len());
        
        service_slice.write_bytes(0, &req.to_bytes(revision_1)).unwrap();
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
        let status = reader.read_val::<CryptoInhdr>().unwrap();

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
        //session create&destroy
        {
            let res = self.create_hash_session(CryptoHashAlgorithm::Sha256, 64);
            debug!("try to create hash session:{:?}", res);

            let res = self.create_mac_session(CryptoMacAlgorithm::CbcMacAes, 16, &[0;16]);
            debug!("try to create mac session:{:?}", res);

            let res = self.create_aead_session(
                CryptoAeadAlgorithm::AeadCcm, CryptoDirection::Encrypt, 16, 16, &[0; 16]);
            debug!("try to create aead session:{:?}", res);

            let res = self.create_akcipher_rsa_session(
                CryptoAkCipherAlgorithm::AkCipherRSA, CryptoRsaPaddingAlgo::RAW, 
                CryptoRsaHashAlgo::NoHash, CryptoAkCipherKeyType::Public, &[0; 64]);
            debug!("try to create akcipher session:{:?}", res);

            let res = self.create_alg_chain_hash_session(
                CryptoCipherAlgorithm::AesEcb, CryptoDirection::Encrypt, CryptoSymAlgChainOrder::CipherThenHash,
                CryptoHashAlgorithm::Sha256, 32, 32, &[0; 32]);
            debug!("try to create alg chain plain session:{:?}", res);
        }

        //Cipher encrypt&decrypt
        {
            //create encrypt session
            let res = 
            self.create_cipher_session(CryptoCipherAlgorithm::AesEcb, 
                                        CryptoDirection::Encrypt, &[1; 16]);
            debug!("try to create cipher session:{:?}", res);
            
            let encrypt_session_id = res.unwrap();
            debug!("encrypt cipher session id: {:?}", encrypt_session_id);
            
            //encrypt service
            let res=
                self.handle_cipher_service_req(
                    CryptoServiceOperation::CipherEncrypt,  CryptoCipherAlgorithm::AesEcb, 
                    encrypt_session_id,  &[0; 16], &[2; 16], 16
                );
            
            let encrypted = res.unwrap();
            debug!("AES ECB encrypted data: {:?}", encrypted);
            
            //create decrypt session
            let res= self.create_cipher_session(CryptoCipherAlgorithm::AesEcb, 
                CryptoDirection::Decrypt, &[1; 16]);
            
            let decrypt_session_id = res.unwrap();
            debug!("decrypt session id: {:?}", decrypt_session_id);
            
            //decrypt service
            let res = 
                self.handle_cipher_service_req(
                    CryptoServiceOperation::CipherDecrypt, CryptoCipherAlgorithm::AesEcb,
                    decrypt_session_id, &[0;16], &encrypted, 16
                );
            
            let decrypted = res.unwrap();
            debug!("AES ECB decrypted data: {:?}", decrypted);
            
            //destroy all session
            let res = self.destroy_cipher_session(encrypt_session_id);
            debug!("try to destroy session {:?} : {:?}", encrypt_session_id, res);
            let res = self.destroy_cipher_session(decrypt_session_id);
            debug!("try to destroy session {:?} : {:?}", decrypt_session_id, res);
        }
    

    }

    fn create_hash_session(&self, algo: CryptoHashAlgorithm, result_len: u32)->Result<i64, CryptoError>{
        debug!("[CRYPTO] trying to create hash session");
    
        let header = CryptoCtrlHeader { 
            opcode: CryptoSessionOperation::HashCreate as i32, 
            algo: algo as _,
            flag: 0, 
            reserved: 0
        };
    
        let req = CryptoSessionRequest{
            header,
            flf: CryptoHashSessionFlf{algo: algo as _, hash_result_len: result_len},
        };
        
        self.create_session(req, &[])
    }

    fn handle_hash_service_req(&self, op : CryptoServiceOperation, algo: CryptoHashAlgorithm, session_id : i64, src_data: &[u8], hash_result_len: i32) -> Result<Vec<u8>, CryptoError> {
        debug!("[CRYPTO] trying to handle hash service request");
        let header = CryptoServiceHeader {
            opcode: op as _,
            algo: algo as _,
            session_id,
            flag : 1,
            padding : 0
        };
        let src_data_len = src_data.len() as i32;
        let flf = CryptoHashDataFlf {
            src_data_len,
            hash_result_len
        };
        let req = CryptoServiceRequest {
            header,
            flf
        };
        self.handle_service(req, src_data, hash_result_len)
    }

    fn handle_hash_service_req_stateless(&self, op : CryptoServiceOperation, algo : CryptoHashAlgorithm, src_data : &[u8], hash_result_len : i32) -> Result<Vec<u8>, CryptoError> {
        debug!("[CRYPTO] trying to handle stateless hash service request");
        let header = CryptoServiceHeader {
            opcode : op as _,
            algo : algo as _,
            session_id : 0,
            flag : 0,
            padding : 0
        };
        let src_data_len = src_data.len() as i32;
        let flf = CryptoHashDataFlfStateless {
            session_algo : algo as _,
            src_data_len,
            hash_result_len,
            reserved : 0
        };
        let req = CryptoServiceRequest {
            header,
            flf
        };
        self.handle_service(req, src_data, hash_result_len)
    }

    fn destroy_hash_session(&self, session_id : i64) -> Result<(), CryptoError> {
        debug!("[CRYPTO] trying to destroy hash session");
        self.destroy_session(CryptoSessionOperation::HashDestroy, session_id)
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

        let req = CryptoSessionRequest{
            header,
            flf: CryptoMacSessionFlf{
                algo: algo as _,
                mac_result_len: result_len,
                auth_key_len: key_len,
                padding: 0
            }
        };

        self.create_session(req, auth_key)
    }

    fn handle_mac_service_req(&self, op : CryptoServiceOperation, algo: CryptoMacAlgorithm, session_id : i64, src_data: &[u8], hash_result_len: i32) -> Result<Vec<u8>, CryptoError> {
        debug!("[CRYPTO] trying to handle mac service request");
        let header = CryptoServiceHeader {
            opcode: op as _,
            algo: algo as _,
            session_id,
            flag : 1,
            padding : 0
        };
        let src_data_len = src_data.len() as i32;
        let flf = CryptoHashDataFlf {
            src_data_len,
            hash_result_len
        };
        let req = CryptoServiceRequest{
            header,
            flf
        };
        self.handle_service(req, src_data, hash_result_len)
    }

    fn handle_mac_service_req_stateless(&self, op : CryptoServiceOperation, algo : CryptoMacAlgorithm, src_data : &[u8], auth_key : &[u8], hash_result_len : i32) -> Result<Vec<u8>, CryptoError> {
        debug!("[CRYPTO] trying to handle stateless mac service request");
        let header = CryptoServiceHeader {
            opcode: op as _,
            algo: algo as _,
            session_id : 0,
            flag : 0,
            padding : 0
        };
        let auth_key_len = auth_key.len() as i32;
        let src_data_len = src_data.len() as i32;
        let flf = CryptoMacDataFlfStateless {
            session_algo : algo as _,
            session_auth_key_len : auth_key_len,
            src_data_len,
            hash_result_len
        };
        let req = CryptoServiceRequest {
            header,
            flf
        };
        let vlf = &[auth_key, src_data].concat();
        self.handle_service(req, vlf, hash_result_len)
    }

    fn destroy_mac_session(&self, session_id : i64) -> Result<(), CryptoError> {
        debug!("[CRYPTO] trying to destroy mac session");
        self.destroy_session(CryptoSessionOperation::MacDestroy, session_id)
    }

    fn create_aead_session(&self, algo: CryptoAeadAlgorithm, op: CryptoDirection, tag_len: i32, aad_len: i32, key: &[u8]) -> Result<i64, CryptoError> {
        debug!("[CRYPTO] trying to create aead session");
        let key_len = key.len() as _;
        let header = CryptoCtrlHeader {
            opcode : CryptoSessionOperation::AeadCreate as i32,
            algo: algo as _,
            flag: 0,
            reserved: 0
        };
        let flf = CryptoAeadSessionFlf {
            algo: algo as _,
            key_len,
            tag_len,
            aad_len,
            op: op as _,
            padding: 0
        };
        let req = CryptoSessionRequest{header, flf};
        self.create_session(req, key)
    }

    fn handle_aead_service_req(&self, op : CryptoServiceOperation, algo : CryptoAeadAlgorithm, session_id : i64, iv: &[u8], src_data: &[u8], aad : &[u8], dst_data_len : i32, tag_len: i32) -> Result<Vec<u8>, CryptoError> {
        debug!("[CRYPTO] trying to handle aead service request");
        let header = CryptoServiceHeader {
            opcode: op as _,
            algo: algo as _,
            session_id,
            flag : 1,
            padding : 0
        };
        let iv_len = iv.len() as i32;
        let src_data_len = src_data.len() as i32;
        let aad_len = aad.len() as i32;
        let flf = CryptoAeadDataFlf {
            iv_len,
            aad_len,
            src_data_len,
            dst_data_len,
            tag_len,
            reserved : 0
        };
        let req = CryptoServiceRequest{
            header,
            flf
        };
        self.handle_service(req, &[iv, src_data, aad].concat(), dst_data_len)
    }

    fn handle_aead_service_req_stateless(&self, op : CryptoServiceOperation, algo : CryptoAeadAlgorithm, key : &[u8], dir : CryptoDirection, iv: &[u8], tag_len: i32, aad: &[u8], src_data: &[u8], dst_data_len : i32) -> Result<Vec<u8>, CryptoError> {
        debug!("[CRYPTO] trying to handle stateless aead service request");
        let header = CryptoServiceHeader {
            opcode: op as _,
            algo : algo as _,
            session_id: 0,
            flag: 0,
            padding: 0
        };
        let key_len = key.len() as i32;
        let iv_len = iv.len() as i32;
        let aad_len = aad.len() as i32;
        let src_data_len = src_data.len() as i32;
        let flf = CryptoAeadDataFlfStateless {
            session_algo : algo as _,
            session_key_len : key_len,
            session_op : dir as _,
            iv_len,
            tag_len,
            src_data_len,
            dst_data_len,
            aad_len
        };
        let req = CryptoServiceRequest {
            header,
            flf
        };
        let vlf = &[key, iv, src_data, aad].concat();
        self.handle_service(req, vlf, dst_data_len)
    }

    fn destroy_aead_session(&self, session_id : i64) -> Result<(), CryptoError> {
        self.destroy_session(CryptoSessionOperation::AeadDestroy, session_id)
    }

    fn create_cipher_session(&self, algo: CryptoCipherAlgorithm, op: CryptoDirection, key: &[u8])->Result<i64, CryptoError>{
        debug!("[CRYPTO] trying to create cipher session");

        let key_len: i32 = key.len() as _;
        let header = CryptoCtrlHeader { 
            opcode: CryptoSessionOperation::CipherCreate as i32, 
            algo: algo as _,
            flag: 0, 
            reserved: 0
        };
    
        let flf = CryptoCipherSessionFlf{
            algo: algo as _, key_len, op: op as _, padding: 0
        };
        let req = CryptoSessionRequest{
            header,
            flf: CryptoSymSessionFlf{
                op_flf: CryptoSymSessionOpFlf{cipher_flf: flf},
                op_type: CryptoSymOpType::Cipher as _,
                padding: 0
            }
        };

        self.create_session(req, key)
    }

    fn create_alg_chain_mac_session(&self, algo: CryptoCipherAlgorithm, op: CryptoDirection, alg_chain_order: CryptoSymAlgChainOrder, mac_algo: CryptoMacAlgorithm, result_len: u32, aad_len: i32, cipher_key: &[u8], auth_key: &[u8])->Result<i64, CryptoError> {
        debug!("[CRYPTO] trying to create alg chain auth session");
        let hash_mode = CryptoSymHashMode::Auth;
        let header = CryptoCtrlHeader { 
            opcode: CryptoSessionOperation::CipherCreate as i32, 
            algo: algo as _,
            flag: 0, 
            reserved: 0
        };
        let key_len: u32 = cipher_key.len() as _;

        let cipher_flf = CryptoCipherSessionFlf{
            algo: algo as _, key_len: key_len as _, op: op as _, padding: 0
        };
        let auth_key_len: u32 = auth_key.len() as _;
        let mac_flf = CryptoMacSessionFlf{
            algo: mac_algo as _, mac_result_len: result_len, auth_key_len, padding: 0
        };
        let flf = 
            CryptoAlgChainSessionFlf::new(
                alg_chain_order, hash_mode, cipher_flf, CryptoAlgChainSessionAlgo{mac_flf}, aad_len
            );
        
        let req = CryptoSessionRequest{
            header, 
            flf: CryptoSymSessionFlf{
                op_flf: CryptoSymSessionOpFlf{alg_chain_flf: flf},
                op_type: CryptoSymOpType::AlgorithmChaining as _,
                padding: 0                
            }
        };

        self.create_session(req, &[cipher_key, auth_key].concat())
    }

    fn create_alg_chain_hash_session(&self, algo: CryptoCipherAlgorithm, op: CryptoDirection, alg_chain_order: CryptoSymAlgChainOrder, hash_algo: CryptoHashAlgorithm, result_len: u32, aad_len: i32, cipher_key: &[u8])->Result<i64, CryptoError> {
        debug!("[CRYPTO] trying to create alg chain plain session");
        let hash_mode = CryptoSymHashMode::Plain;
        let header = CryptoCtrlHeader { 
            opcode: CryptoSessionOperation::CipherCreate as i32, 
            algo: algo as _,
            flag: 0, 
            reserved: 0
        };
        let key_len: u32 = cipher_key.len() as _;
        let cipher_flf = CryptoCipherSessionFlf{
            algo: algo as _, key_len: key_len as _, op: op as _, padding: 0 
        };
        let hash_flf = CryptoHashSessionFlf{algo: hash_algo as _, hash_result_len: result_len};
        let flf = CryptoAlgChainSessionFlf::new(
            alg_chain_order, hash_mode, cipher_flf, CryptoAlgChainSessionAlgo{hash_flf}, aad_len
        );
        let req = CryptoSessionRequest{
            header, 
            flf: CryptoSymSessionFlf{
                op_flf: CryptoSymSessionOpFlf{alg_chain_flf: flf},
                op_type: CryptoSymOpType::AlgorithmChaining as _,
                padding: 0                
            }
        };

        self.create_session(req, cipher_key)
    }

    fn handle_cipher_service_req(&self, op : CryptoServiceOperation, algo: CryptoCipherAlgorithm, session_id : i64, iv : &[u8], src_data : &[u8], dst_data_len : i32) -> Result<Vec<u8>, CryptoError> {
        debug!("[CRYPTO] trying to handle cipher service request");
        let header = CryptoServiceHeader {
            opcode : op as _,
            algo : algo as _,
            session_id,
            flag : 1, // VIRTIO_CRYPTO_FLAG_SESSION_MODE
            padding : 0
        };
        let src_data_len = src_data.len() as i32;
        let iv_len = iv.len() as i32;
        let flf = CryptoCipherDataFlf::new(iv_len, src_data_len, dst_data_len);
        let req = CryptoServiceRequest {
            header,
            flf : CryptoSymDataFlf {
                op_type_flf : CryptoSymDataOpFlf{ cipher_flf : flf},
                op_type : CryptoSymOpType::Cipher as _,
                padding : 0
            }
        };

        let vlf = &[iv, src_data].concat();

        let dst_data = self.handle_service(req, vlf, dst_data_len);
        dst_data

    }

    fn handle_cipher_service_req_stateless(&self, op : CryptoServiceOperation, algo : CryptoCipherAlgorithm, key: &[u8], dir : CryptoDirection, iv : &[u8], src_data: &[u8], dst_data_len : i32) -> Result<Vec<u8>, CryptoError> {
        debug!("[CRYPTO] trying to handle stateless cipher service request");
        let header = CryptoServiceHeader {
            opcode : op as _,
            algo : algo as _,
            session_id : 0,
            flag : 0,
            padding : 0
        };
        let key_len = key.len() as i32;
        let iv_len = iv.len() as i32;
        let src_data_len = src_data.len() as i32;
        let cipher_flf = CryptoCipherDataFlfStateless {
            session_algo : algo as _,
            session_key_len : key_len,
            session_op : dir as _,
            iv_len,
            src_data_len,
            dst_data_len
        };
        let flf = CryptoSymDataFlfStateless {
            op_type_flf : CryptoSymDataOpFlfStateless {cipher_flf},
            op_type : CryptoSymOpType::Cipher as _
        };
        let req = CryptoServiceRequest {
            header,
            flf
        };
        let vlf = &[key, iv, src_data].concat();
        self.handle_service(req, vlf, dst_data_len)
    }

    fn handle_alg_chain_service_req(&self, op : CryptoServiceOperation, algo: CryptoCipherAlgorithm, session_id: i64, iv : &[u8], src_data : &[u8], dst_data_len: i32, cipher_start_src_offset: i32, len_to_cipher: i32, hash_start_src_offset: i32, len_to_hash: i32, aad_len: i32, hash_result_len: i32) -> Result<(Vec<u8>, Vec<u8>), CryptoError> {
        debug!("[CRYPTO] trying to handle alg chain service request");
        let header = CryptoServiceHeader {
            opcode : op as _,
            algo : algo as _,
            session_id,
            flag : 1, // VIRTIO_CRYPTO_FLAG_SESSION_MODE
            padding : 0
        };
        let src_data_len = src_data.len() as i32;
        let iv_len = iv.len() as i32;
        let flf = CryptoAlgChainDataFlf::new(
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
        let req = CryptoServiceRequest {
            header,
            flf : CryptoSymDataFlf {
                op_type_flf : CryptoSymDataOpFlf { alg_chain_flf : flf},
                op_type : CryptoSymOpType::AlgorithmChaining as _,
                padding : 0
            }
        };

        let vlf = &[iv, src_data].concat();

        let dst_data = self.handle_service(req, vlf, dst_data_len + hash_result_len);
        match dst_data {
            Ok(data) => {
                let (fi, sc) = data.split_at(dst_data_len as _);
                Ok((fi.to_vec(), sc.to_vec()))
            }
            Err(err) => Err(err)
        }
    }

    fn handle_alg_chain_service_req_stateless(
            &self, op : CryptoServiceOperation, algo : CryptoCipherAlgorithm, 
            alg_chain_order: CryptoSymAlgChainOrder, aad : &[u8], 
            cipher_key : &[u8], dir : CryptoDirection, 
            hash_algo: i32, auth_key: &[u8], hash_mode : CryptoSymHashMode, 
            iv : &[u8], src_data : &[u8], dst_data_len : i32, 
            cipher_start_src_offset: i32, len_to_cipher: i32, hash_start_src_offset: i32, len_to_hash: i32, hash_result_len: i32
        )->Result<(Vec<u8>, Vec<u8>), CryptoError> {
        debug!("[CRYPTO] trying to handle stateless alg chain service request");
        let header = CryptoServiceHeader {
            opcode : op as _,
            algo : algo as _,
            session_id : 0,
            flag : 0,
            padding : 0
        };
        let aad_len = aad.len() as i32;
        let key_len = cipher_key.len() as i32;
        let hash_auth_key_len = auth_key.len() as i32;
        let iv_len = iv.len() as i32;
        let src_data_len = src_data.len() as i32;
        let alg_chain_flf = CryptoAlgChainDataFlfStateless {
            session_alg_chain_order : alg_chain_order as _,
            session_aad_len : aad_len,
            session_cipher_algo : algo as _,
            session_cipher_key_len : key_len,
            session_cipher_op : dir as _,
            session_hash_algo : hash_algo as _,
            session_hash_auth_key_len : hash_auth_key_len, 
            session_hash_mode : hash_mode as _,
            iv_len,
            src_data_len,
            dst_data_len,
            cipher_start_src_offset,
            len_to_cipher,
            hash_start_src_offset,
            len_to_hash,
            aad_len,
            hash_result_len,
            reserved : 0
        };
        let flf = CryptoSymDataFlfStateless {
            op_type_flf : CryptoSymDataOpFlfStateless {alg_chain_flf},
            op_type : CryptoSymOpType::Cipher as _
        };
        let req = CryptoServiceRequest {
            header,
            flf
        };
        let vlf = &[cipher_key, auth_key, iv, aad, src_data].concat();
        let dst_data = self.handle_service(req, vlf, dst_data_len + hash_result_len);
        match dst_data {
            Ok(data) => {
                let (fi, sc) = data.split_at(dst_data_len as _);
                Ok((fi.to_vec(), sc.to_vec()))
            }
            Err(err) => Err(err)
        }

    }

    fn destroy_cipher_session(&self, session_id: i64) -> Result<(), CryptoError> {
        debug!("[CRYPTO] trying to destroy cipher session");
        self.destroy_session(CryptoSessionOperation::CipherDestroy, session_id)
    
    }

    fn create_akcipher_rsa_session(&self, algo: CryptoAkCipherAlgorithm,
                                   padding_algo: CryptoRsaPaddingAlgo,
                                   hash_algo: CryptoRsaHashAlgo,
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
        let para = CryptoRsaPara {
            padding_algo: padding_algo as _,
            hash_algo: hash_algo as _,
        };
        let algo_flf = CryptoAkCipherAlgoFlf { rsa: para };
        let flf = CryptoAkCipherSessionFlf{
            algo: algo as _, key_type: key_type as _, key_len, algo_flf
        };
        let req = CryptoSessionRequest {
            header,
            flf,
        };

        self.create_session(req, &key)
    }

    fn create_akcipher_ecdsa_session(&self, algo: CryptoAkCipherAlgorithm,
                                     curve_id: CryptoEcdsaCurve,
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
        let para = CryptoEcdsaPara {
            curve_id: curve_id as _,
        };
        let algo_flf = CryptoAkCipherAlgoFlf { ecdsa: para };
        let flf = CryptoAkCipherSessionFlf{
            algo: algo as _, key_type: key_type as _, key_len, algo_flf
        };
        let req = CryptoSessionRequest {
            header,
            flf,
        };

        self.create_session(req, &key)
    }

    fn handle_akcipher_ecdsa_service_req_stateless(
            &self, op : CryptoServiceOperation, algo : CryptoAkCipherAlgorithm, key_type : CryptoAkCipherKeyType, akcipher_key : &[u8], 
            curve_id: CryptoEcdsaCurve, 
            src_data : &[u8], dst_data_len : i32
        ) -> Result<Vec<u8>, CryptoError> {
        let header = CryptoServiceHeader {
            opcode : op as _,
            algo : algo as _,
            session_id : 0,
            flag : 0,
            padding : 0
        };
        let ecdsa_flf = CryptoEcdsaPara {
            curve_id : curve_id as _
        };
        let key_len = akcipher_key.len() as i32;
        let src_data_len = src_data.len() as i32;
        let flf = CryptoAkcipherDataFlfStateless {
            session_algo : algo as _,
            session_key_type : key_type as _,
            session_key_len : key_len, 
            session_u : CryptoAkCipherAlgoFlf {ecdsa : ecdsa_flf},
            src_data_len,
            dst_data_len
        };
        let req = CryptoServiceRequest {
            header,
            flf
        };
        let vlf = &[akcipher_key, src_data].concat();
        self.handle_service(req, vlf, dst_data_len)
    }

    fn handle_akcipher_service_req(&self, op : CryptoServiceOperation, algo: CryptoAkCipherAlgorithm, session_id: i64, src_data : &[u8], dst_data_len : i32) -> Result<Vec<u8>, CryptoError> {
        debug!("[CRYPTO] trying to handle akcipher service request");
        let header = CryptoServiceHeader {
            opcode : op as _,
            algo : algo as _,
            session_id,
            flag : 1, // VIRTIO_CRYPTO_FLAG_SESSION_MODE
            padding : 0
        };
        let src_data_len = src_data.len() as i32;
        let flf = CryptoAkcipherDataFlf {
            src_data_len,
            dst_data_len
        };
        let req = CryptoServiceRequest {
            header,
            flf
        };

        let vlf = src_data;

        let dst_data = self.handle_service(req, vlf, dst_data_len);
        dst_data
    }

    fn handle_akcipher_rsa_service_req_stateless(
            &self, op : CryptoServiceOperation, algo : CryptoAkCipherAlgorithm, key_type : CryptoAkCipherKeyType, akcipher_key : &[u8], 
            padding_algo: CryptoRsaPaddingAlgo, hash_algo: CryptoRsaHashAlgo,
            src_data : &[u8], dst_data_len : i32
        ) -> Result<Vec<u8>, CryptoError> {
        let header = CryptoServiceHeader {
            opcode : op as _,
            algo : algo as _,
            session_id : 0,
            flag : 0,
            padding : 0
        };
        let rsa_flf = CryptoRsaPara {
            padding_algo : padding_algo as _,
            hash_algo : hash_algo as _
        };
        let key_len = akcipher_key.len() as i32;
        let src_data_len = src_data.len() as i32;
        let flf = CryptoAkcipherDataFlfStateless {
            session_algo : algo as _,
            session_key_type : key_type as _,
            session_u : CryptoAkCipherAlgoFlf {rsa : rsa_flf},
            session_key_len : key_len,
            src_data_len,
            dst_data_len
        };
        let req = CryptoServiceRequest {
            header,
            flf
        };
        let vlf = &[akcipher_key, src_data].concat();
        self.handle_service(req, vlf, dst_data_len)
    }

    fn destroy_akcipher_session(&self, session_id: i64) -> Result<(), CryptoError> {
        debug!("[CRYPTO] trying to destroy akcipher session");
        self.destroy_session(CryptoSessionOperation::AkCipherDestroy, session_id)
    }
}