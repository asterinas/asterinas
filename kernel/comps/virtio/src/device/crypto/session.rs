
use core::hash;

use alloc::vec::Vec;
use aster_crypto::{CryptoCipherAlgorithm, CryptoError, CryptoHashAlgorithm, CryptoOperation};
use ostd::Pod;

enum CryptoService{
    Cipher = 0,
    Hash = 1,
    Mac = 2,
    Aead = 3,
    AkCipher = 4,
}

pub enum VirtioCryptoStatus { 
    Ok = 0,             // success
    Err = 1,            // any failure not mentioned above occurs
    BadMsg = 2,         // authentication failed (only when AEAD decryption)
    NotSupp = 3,        // operation or algorithm is unsupported
    InvSess = 4,        // invalid session ID when executing crypto operations
    NoSpc = 5,          // no free session ID.
    KeyReject = 6,      // signature verification failed (only when AKCIPHER verification)
}

impl TryFrom<i32> for VirtioCryptoStatus {
    type Error = CryptoError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Ok),
            1 => Ok(Self::Err),
            2 => Ok(Self::BadMsg),
            3 => Ok(Self::NotSupp),
            4 => Ok(Self::InvSess),
            5 => Ok(Self::NoSpc),
            _ => Err(CryptoError::UnknownError),
        }
    }
}

impl VirtioCryptoStatus{
    pub fn get_or_error<T>(&self, val: T)->Result<T, CryptoError>{
        match self {
            VirtioCryptoStatus::Ok => Ok(val),
            VirtioCryptoStatus::Err => Err(CryptoError::UnknownError),
            VirtioCryptoStatus::BadMsg => Err(CryptoError::BadMessage),
            VirtioCryptoStatus::NotSupp => Err(CryptoError::NotSupport),
            VirtioCryptoStatus::InvSess => Err(CryptoError::InvalidSession),
            VirtioCryptoStatus::NoSpc => Err(CryptoError::NoFreeSession),
            VirtioCryptoStatus::KeyReject => Err(CryptoError::KeyReject)
        }
    }
}

const fn crypto_services_opcode(service: CryptoService, op: i32)-> i32{
    ((service as i32) << 8) | op
}

#[derive(Debug, Clone, Copy)]
#[repr(i32)]
pub enum CryptoSessionOperation{
    CipherCreate = crypto_services_opcode(CryptoService::Cipher, 0x02),
    CipherDestroy = crypto_services_opcode(CryptoService::Cipher, 0x03),
    HashCreate = crypto_services_opcode(CryptoService::Hash, 0x02),
    HashDestroy = crypto_services_opcode(CryptoService::Hash, 0x03),
    MacCreate = crypto_services_opcode(CryptoService::Mac, 0x02),
    MacDestroy = crypto_services_opcode(CryptoService::Mac, 0x03),
    AeadCreate = crypto_services_opcode(CryptoService::Aead, 0x02),
    AeadDestroy = crypto_services_opcode(CryptoService::Aead, 0x03),
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
    pub fn to_bytes(&self, padding: bool) -> Vec<u8> {
        <Self as Pod>::as_bytes(&self).to_vec()
    }
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct VirtioCryptoSessionInput{
    pub session_id: i64,
    pub status: i32,
    pub padding: i32,
}

impl VirtioCryptoSessionInput{
    pub fn get_result(&self)->Result<i64, CryptoError>{
        match VirtioCryptoStatus::try_from(self.status){
            Ok(code) => code.get_or_error(self.session_id),
            Err(err) => Err(err)
        }
    }
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoHashSessionReq {
	pub header: CryptoCtrlHeader,
	pub flf: VirtioCryptoHashCreateSessionFlf,
    pub padding: [i32; 12]
}

enum CryptoSymOp{
    None = 0,
    Cipher = 1,
    AlgorithmChaining = 2,
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoCipherSessionReq {
	pub header: CryptoCtrlHeader,
	pub flf: VirtioCryptoCipherCreateSessionFlf,
    pub op_type: i32,
    pub padding: i32,
}

impl CryptoCipherSessionReq{
    pub fn new(header: CryptoCtrlHeader, algo: CryptoCipherAlgorithm, key_len: i32, op: CryptoOperation)->Self{
        Self { 
            header, 
            flf: VirtioCryptoCipherCreateSessionFlf::new(algo, key_len, op), 
            op_type: CryptoSymOp::Cipher as _, 
            padding: 0
        }
    }
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct VirtioCryptoCipherCreateSessionFlf {
    pub algo: i32,
    pub key_len: i32,
    pub op: i32,
    pub padding: u32,
    pub long_padding: [i8; 32],
}

impl VirtioCryptoCipherCreateSessionFlf{
    pub fn new(algo: CryptoCipherAlgorithm, key_len: i32, op: CryptoOperation)->Self{
        Self { 
            algo: algo as _, 
            key_len, 
            op: op as _, 
            padding: 0,
            long_padding: [0; 32], 
        }
    }
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct VirtioCryptoHashCreateSessionFlf {
    pub algo: i32,
    pub hash_result_len: u32
}

impl VirtioCryptoHashCreateSessionFlf{
    pub fn new(algo: CryptoHashAlgorithm, result_len: u32)->Self{
        Self { 
            algo: algo as _,
            hash_result_len: result_len
        }
    }
}

// #[derive(Debug, Pod, Clone, Copy)]
// #[repr(C)]
// pub struct VirtioCryptoDestroySessionPara {
//     pub session_id : i64
// }
// #[derive(Debug, Pod, Clone, Copy)]
// #[repr(C)]
// pub struct VirtioCryptoDestroySessionFlf {
//     pub para : VirtioCryptoDestroySessionPara,
//     pub padding : [i32 ; 12]
// }

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct VirtioCryptoDestroySessionFlf {
    pub session_id : i64
}


impl VirtioCryptoDestroySessionFlf {
    pub fn to_bytes(&self, padding: bool) -> Vec<u8> {
        let res = <Self as Pod>::as_bytes(&self);
        let mut vec = Vec::from(res);
        if padding {
            vec.resize(56, 0);
        }
        vec
    }
}
#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct VirtioCryptoDestroySessionInput {
    pub status : u8
}

impl VirtioCryptoDestroySessionInput {
    pub fn get_result(&self) -> Result<u8, CryptoError> {
        match VirtioCryptoStatus::try_from(self.status as i32){
            Ok(code) => code.get_or_error(self.status),
            Err(err) => Err(err)
        }
    }
}
#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoDestroySessionReq {
    pub header: CryptoCtrlHeader,
	pub flf: VirtioCryptoDestroySessionFlf,
}

impl CryptoDestroySessionReq {
    pub fn to_bytes(&self, padding: bool) -> Vec<u8> {
            // let res = <self as Pod>::as_bytes(&self);
        let header_bytes = self.header.to_bytes(padding);
        let flf_bytes = self.flf.to_bytes(padding);
        return [header_bytes, flf_bytes].concat();
    }
}
