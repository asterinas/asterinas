// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;
use aster_crypto::*;
use ostd::Pod;

pub enum CryptoStatus { 
    Ok = 0,             // success
    Err = 1,            // any failure not mentioned above occurs
    BadMsg = 2,         // authentication failed (only when AEAD decryption)
    NotSupp = 3,        // operation or algorithm is unsupported
    InvSess = 4,        // invalid session ID when executing crypto operations
    NoSpc = 5,          // no free session ID.
    KeyReject = 6,      // signature verification failed (only when AKCIPHER verification)
}

impl TryFrom<i32> for CryptoStatus {
    type Error = CryptoError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Ok),
            1 => Ok(Self::Err),
            2 => Ok(Self::BadMsg),
            3 => Ok(Self::NotSupp),
            4 => Ok(Self::InvSess),
            5 => Ok(Self::NoSpc),
            6 => Ok(Self::KeyReject),
            _ => Err(CryptoError::UnknownError),
        }
    }
}

impl CryptoStatus{
    pub fn get_or_error<T>(&self, val: T)->Result<T, CryptoError>{
        match self {
            CryptoStatus::Ok => Ok(val),
            CryptoStatus::Err => Err(CryptoError::UnknownError),
            CryptoStatus::BadMsg => Err(CryptoError::BadMessage),
            CryptoStatus::NotSupp => Err(CryptoError::NotSupport),
            CryptoStatus::InvSess => Err(CryptoError::InvalidSession),
            CryptoStatus::NoSpc => Err(CryptoError::NoFreeSession),
            CryptoStatus::KeyReject => Err(CryptoError::KeyReject)
        }
    }
}

/*
    Auto Padding
*/

pub trait AutoPadding: Pod{
    const PADDING_BYTES: usize = 0;

    fn to_bytes(&self, revision_1: bool)->Vec<u8>{
        let res = <Self as Pod>::as_bytes(&self);
        let mut vec = Vec::from(res);
        if !revision_1 {
            vec.resize(Self::PADDING_BYTES, 0);
        }
        vec        
    }
}

/*
    Crypto Session Specification
*/
pub trait CtrlFlfPadding: AutoPadding {
    const PADDING_BYTES: usize = 56;
}

pub struct CryptoSessionRequest<T>{
    pub header: CryptoCtrlHeader,
    pub flf: T
}

impl<T: AutoPadding> CryptoSessionRequest<T>{
    pub fn to_bytes(&self, revision_1: bool)->Vec<u8>{
        let header_bytes = self.header.to_bytes();
        let flf_bytes = self.flf.to_bytes(revision_1);
        return [header_bytes, flf_bytes].concat();   
    }

    pub fn len(&self)->usize{
        return size_of::<CryptoCtrlHeader>() + size_of::<T>();
    }
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoCtrlHeader{
    pub opcode: i32,
    pub algo: i32,
    pub flag: i32,
    pub reserved: i32,
}

impl CryptoCtrlHeader {
    pub fn to_bytes(&self) -> Vec<u8> {
        <Self as Pod>::as_bytes(&self).to_vec()
    }
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoHashSessionFlf {
    pub algo: i32,
    pub hash_result_len: u32
}

impl AutoPadding for CryptoHashSessionFlf {}
impl CtrlFlfPadding for CryptoHashSessionFlf {}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoMacSessionFlf{
    pub algo: i32,
    pub mac_result_len: u32,
    pub auth_key_len: u32,
    pub padding: i32,
}

impl AutoPadding for CryptoMacSessionFlf {}
impl CtrlFlfPadding for CryptoMacSessionFlf {}

#[derive(Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoAeadSessionFlf{
    pub algo : i32,
    pub key_len : i32,
    pub tag_len : i32,
    pub aad_len : i32,
    pub op : i32,
    pub padding : i32
}

impl AutoPadding for CryptoAeadSessionFlf {}
impl CtrlFlfPadding for CryptoAeadSessionFlf {}

#[derive(Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoSymSessionFlf{
    pub op_flf: CryptoSymSessionOpFlf,
    pub op_type: i32,
    pub padding: i32
}

impl AutoPadding for CryptoSymSessionFlf {}
impl CtrlFlfPadding for CryptoSymSessionFlf {}

#[derive(Pod, Clone, Copy)]
#[repr(C)]
pub union CryptoSymSessionOpFlf {
    pub cipher_flf : CryptoCipherSessionFlf,
    pub alg_chain_flf : CryptoAlgChainSessionFlf
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoCipherSessionFlf {
    pub algo: i32,
    pub key_len: i32,
    pub op: i32,
    pub padding: u32
}

#[derive(Pod, Clone, Copy)]
#[repr(C)]
pub union CryptoAlgChainSessionAlgo {
    pub hash_flf: CryptoHashSessionFlf,
    pub mac_flf: CryptoMacSessionFlf,
    pub padding: [u8; 16]
}

#[derive(Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoAlgChainSessionFlf {
    pub alg_chain_order : i32,
    pub hash_mode : i32,
    pub cipher_hdr : CryptoCipherSessionFlf,
    pub algo_flf : CryptoAlgChainSessionAlgo,
    pub aad_len : i32,
    pub padding : i32
}

impl CryptoAlgChainSessionFlf {
    pub fn new(alg_chain_order: CryptoSymAlgChainOrder, hash_mode: CryptoSymHashMode, cipher_hdr: CryptoCipherSessionFlf,
               algo_flf: CryptoAlgChainSessionAlgo, aad_len: i32) -> Self {
            Self {
                alg_chain_order: alg_chain_order as _,
                hash_mode: hash_mode as _,
                cipher_hdr,
                algo_flf,
                aad_len,
                padding: 0
            }
        }
}

#[derive(Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoAkCipherSessionFlf {
    pub algo: i32,
    pub key_type: i32,
    pub key_len: u32,
    pub algo_flf: CryptoAkCipherAlgoFlf,
}

impl AutoPadding for CryptoAkCipherSessionFlf {}
impl CtrlFlfPadding for CryptoAkCipherSessionFlf {}

#[derive(Pod, Clone, Copy)]
#[repr(C)]
pub union CryptoAkCipherAlgoFlf {
    pub rsa: CryptoRsaPara,
    pub ecdsa: CryptoEcdsaPara,
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoRsaPara {
    pub padding_algo: i32,
    pub hash_algo: i32,
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoEcdsaPara {
    pub curve_id: i32,
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoSessionInput{
    pub session_id: i64,
    pub status: i32,
    pub padding: i32,
}

impl CryptoSessionInput{
    pub fn get_result(&self)->Result<i64, CryptoError>{
        match CryptoStatus::try_from(self.status){
            Ok(code) => code.get_or_error(self.session_id),
            Err(err) => Err(err)
        }
    }
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoDestroySessionFlf {
    pub session_id : i64
}

impl AutoPadding for CryptoDestroySessionFlf {}
impl CtrlFlfPadding for CryptoDestroySessionFlf {}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoDestroySessionInput {
    pub status : u8
}

impl CryptoDestroySessionInput {
    pub fn get_result(&self) -> Result<u8, CryptoError> {
        match CryptoStatus::try_from(self.status as i32){
            Ok(code) => code.get_or_error(self.status),
            Err(err) => Err(err)
        }
    }
}

/*
    Crypto Service
*/

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoServiceHeader {
    pub opcode : i32,
    pub algo : i32,
    pub session_id : i64,
    pub flag : i32,
    pub padding : i32
}

impl CryptoServiceHeader {
    pub fn to_bytes(&self) -> Vec<u8> {
        Vec::from(<Self as Pod>::as_bytes(&self))
    }
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoInhdr {
    pub status : u8
}

impl CryptoInhdr {
    pub fn get_result(&self) -> Result<u8, CryptoError> {
        match CryptoStatus::try_from(self.status as i32){
            Ok(code) => code.get_or_error(self.status),
            Err(err) => Err(err)
        }
    }
}

pub trait DataFlfPadding: AutoPadding {
    const PADDING_BYTES: usize = 48;
}

pub struct CryptoServiceRequest<T> {
    pub header: CryptoServiceHeader,
    pub flf: T
}

impl<T: DataFlfPadding> CryptoServiceRequest<T> {
    pub fn to_bytes(&self, revision_1: bool)->Vec<u8>{
        let header_bytes = self.header.to_bytes();
        let flf_bytes = self.flf.to_bytes(revision_1);
        return [header_bytes, flf_bytes].concat();   
    }

    pub fn len(&self)->usize{
        return size_of::<CryptoServiceHeader>() + size_of::<T>();
    }
}

#[derive(Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoHashDataFlf {
    pub src_data_len : i32,
    pub hash_result_len : i32 
}

impl AutoPadding for CryptoHashDataFlf {}
impl DataFlfPadding for CryptoHashDataFlf {}

#[derive(Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoSymDataFlf {
    pub op_type_flf : CryptoSymDataOpFlf,
    pub op_type : i32,
    pub padding : i32
}

impl AutoPadding for CryptoSymDataFlf {}
impl DataFlfPadding for CryptoSymDataFlf {}

#[derive(Pod, Clone, Copy)]
#[repr(C)]
pub union CryptoSymDataOpFlf {
    pub cipher_flf : CryptoCipherDataFlf,
    pub alg_chain_flf : CryptoAlgChainDataFlf
} 

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoCipherDataFlf {
    pub iv_len : i32,
    pub src_data_len : i32,
    pub dst_data_len : i32,
    pub padding : i32
}

impl CryptoCipherDataFlf {
    pub fn new(iv_len : i32, src_data_len : i32, dst_data_len : i32) -> Self {
        Self {
            iv_len,
            src_data_len,
            dst_data_len,
            padding : 0
        }
    }
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoAlgChainDataFlf {
    pub iv_len : i32,
    pub src_data_len : i32,
    pub dst_data_len : i32,
    pub cipher_start_src_offset : i32,
    pub len_to_cipher : i32,
    pub hash_start_src_offset : i32,
    pub len_to_hash : i32,
    pub aad_len : i32,
    pub hash_result_len : i32,
    pub reserved : i32
}

impl CryptoAlgChainDataFlf {
    pub fn new(iv_len : i32, 
        src_data_len : i32, 
        dst_data_len : i32, 
        cipher_start_src_offset : i32, 
        len_to_cipher : i32, 
        hash_start_src_offset : i32, 
        len_to_hash : i32, 
        aad_len : i32, 
        hash_result_len : i32) -> Self {
        Self {
            iv_len,
            src_data_len,
            dst_data_len,
            cipher_start_src_offset,
            len_to_cipher,
            hash_start_src_offset,
            len_to_hash,
            aad_len,
            hash_result_len,
            reserved: 0
        }
    }
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoAeadDataFlf {
    pub iv_len : i32,
    pub aad_len : i32,
    pub src_data_len : i32,
    pub dst_data_len : i32,
    pub tag_len : i32,
    pub reserved : i32
}

impl AutoPadding for CryptoAeadDataFlf {}
impl DataFlfPadding for CryptoAeadDataFlf {}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoAkcipherDataFlf {
    pub src_data_len : i32,
    pub dst_data_len : i32
}

impl AutoPadding for CryptoAkcipherDataFlf {}
impl DataFlfPadding for CryptoAkcipherDataFlf {}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoHashDataFlfStateless {
    pub session_algo : i32,

    pub src_data_len : i32,
    pub hash_result_len : i32,
    pub reserved : i32
}

impl AutoPadding for CryptoHashDataFlfStateless {}
impl DataFlfPadding for CryptoHashDataFlfStateless {
    const PADDING_BYTES: usize = size_of::<CryptoHashDataFlfStateless>();
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoMacDataFlfStateless {
    pub session_algo : i32,
    pub session_auth_key_len : i32,

    pub src_data_len : i32,
    pub hash_result_len : i32
}

impl AutoPadding for CryptoMacDataFlfStateless {}
impl DataFlfPadding for CryptoMacDataFlfStateless {
    const PADDING_BYTES: usize = size_of::<CryptoMacDataFlfStateless>();
}

#[derive(Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoSymDataFlfStateless {
    pub op_type_flf : CryptoSymDataOpFlfStateless,
    pub op_type : i32
}

impl AutoPadding for CryptoSymDataFlfStateless {}
impl DataFlfPadding for CryptoSymDataFlfStateless {
    const PADDING_BYTES: usize = size_of::<CryptoSymDataFlfStateless>();
}


#[derive(Pod, Clone, Copy)]
#[repr(C)]
pub union CryptoSymDataOpFlfStateless {
    pub cipher_flf : CryptoCipherDataFlfStateless,
    pub alg_chain_flf : CryptoAlgChainDataFlfStateless
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoCipherDataFlfStateless {
    pub session_algo : i32,
    pub session_key_len : i32,
    pub session_op : i32,

    pub iv_len : i32,
    pub src_data_len : i32,
    pub dst_data_len : i32
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoAlgChainDataFlfStateless {
    pub session_alg_chain_order : i32,
    pub session_aad_len : i32,
    pub session_cipher_algo : i32,
    pub session_cipher_key_len : i32,
    pub session_cipher_op : i32,
    pub session_hash_algo : i32,
    pub session_hash_auth_key_len : i32,
    pub session_hash_mode : i32,
    
    pub iv_len : i32,
    pub src_data_len : i32,
    pub dst_data_len : i32,
    pub cipher_start_src_offset : i32,
    pub len_to_cipher : i32,
    pub hash_start_src_offset : i32,
    pub len_to_hash : i32,
    pub aad_len : i32,
    pub hash_result_len : i32,
    pub reserved : i32
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoAeadDataFlfStateless {
    pub session_algo : i32,
    pub session_key_len : i32,
    pub session_op : i32,

    pub iv_len : i32,
    pub tag_len : i32,
    pub aad_len : i32,
    pub src_data_len : i32,
    pub dst_data_len : i32
}

impl AutoPadding for CryptoAeadDataFlfStateless {}
impl DataFlfPadding for CryptoAeadDataFlfStateless {
    const PADDING_BYTES: usize = size_of::<CryptoAeadDataFlfStateless>();
}

#[derive(Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoAkcipherDataFlfStateless {
    pub session_algo : i32,
    pub session_key_type : i32,
    pub session_key_len : i32,
    pub session_u : CryptoAkCipherAlgoFlf,

    pub src_data_len : i32,
    pub dst_data_len : i32
}

impl AutoPadding for CryptoAkcipherDataFlfStateless {}
impl DataFlfPadding for CryptoAkcipherDataFlfStateless {
    const PADDING_BYTES: usize = size_of::<CryptoAkcipherDataFlfStateless>();
}