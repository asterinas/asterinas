
use core::hash;

use alloc::vec::Vec;
use aster_crypto::{CryptoCipherAlgorithm, CryptoError, CryptoHashAlgorithm, CryptoMacAlgorithm, CryptoSymAlgChainOrder, CryptoSymHashMode, CryptoOperation, CryptoSymOp};
use ostd::Pod;

pub enum CryptoService{
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

pub const fn crypto_services_opcode(service: CryptoService, op: i32)-> i32{
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

pub trait CryptoSessionRequest: Pod{
    fn to_bytes(&self, padding: bool)->Vec<u8>;
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
	pub flf: VirtioCryptoHashSessionFlf,
}

impl CryptoSessionRequest for CryptoHashSessionReq{
    fn to_bytes(&self, padding: bool)->Vec<u8> {
        let header_bytes = self.header.to_bytes(padding);
        let flf_bytes = self.flf.to_bytes(padding);
        return [header_bytes, flf_bytes].concat();        
    }
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct VirtioCryptoHashSessionFlf {
    pub algo: i32,
    pub hash_result_len: u32
}

impl VirtioCryptoHashSessionFlf{
    pub fn new(algo: CryptoHashAlgorithm, result_len: u32)->Self{
        Self { 
            algo: algo as _,
            hash_result_len: result_len
        }
    }
}

impl VirtioCryptoHashSessionFlf {
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
pub struct CryptoMacSessionReq{
    pub header: CryptoCtrlHeader,
    pub flf: VirtioCryptoMacSessionFlf
}

impl CryptoSessionRequest for CryptoMacSessionReq{
    fn to_bytes(&self, padding: bool)->Vec<u8> {
        let header_bytes = self.header.to_bytes(padding);
        let flf_bytes = self.flf.to_bytes(padding);
        return [header_bytes, flf_bytes].concat();      
    }
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct VirtioCryptoMacSessionFlf{
    pub algo: i32,
    pub mac_result_len: u32,
    pub auth_key_len: u32,
    pub padding: i32,
}

impl VirtioCryptoMacSessionFlf{

    pub fn new(algo: CryptoMacAlgorithm, mac_result_len: u32, auth_key_len: u32)->Self{
        Self{algo: algo as _, mac_result_len, auth_key_len, padding: 0}
    }

    pub fn to_bytes(&self, padding: bool) -> Vec<u8> {
        let res = <Self as Pod>::as_bytes(&self);
        let mut vec = Vec::from(res);
        if padding {
            vec.resize(56, 0);
        }
        vec
    }  
}

#[derive(Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoCipherSessionReq {
	pub header: CryptoCtrlHeader,
	pub flf: VirtioCryptoSymCreateSessionFlf,
    pub op_type: i32,
    pub padding: i32,
}

impl CryptoCipherSessionReq{
    // pub fn new(header: CryptoCtrlHeader, algo: CryptoCipherAlgorithm, key_len: i32, op: CryptoOperation, sym_op: CryptoSymOp, hash_op: CryptoSymHashMode)->Self{
    //     if sym_op == CryptoSymOp::Cipher {
    //         Self { 
    //             header, 
    //             flf: VirtioCryptoCipherSessionFlf::new(algo, key_len, op), 
    //             op_type: CryptoSymOp::Cipher as _, 
    //             padding: 0
    //         }
    //     }
    //     else if sym_op == CryptoSymOp::AlgorithmChaining {
    //         let cipher_flf = VirtioCryptoCipherSessionFlf::new(algo, key_len, op);

    //         Self {
    //             header,
    //             flf: VirtioCryptoAlgChainSessionFlf {
    //                 alg_chain_order
    //             }
    //         }
    //     }
    //     else if sym_op == CryptoSymOp::None {
    //         Self {
    //             header,
    //             flf: VirtioCryptoCipherSessionFlf::new(algo, key_len, op),
    //             op_type: CryptoSymOp::None as _,
    //             padding: 0
    //         }
    //     }
        
    // }
    pub fn new(header: CryptoCtrlHeader, flf: VirtioCryptoSymCreateSessionFlf, sym_op: CryptoSymOp) -> Self {
        Self {
            header,
            flf,
            op_type : sym_op as _,
            padding: 0
        }
    }
}

impl CryptoSessionRequest for CryptoCipherSessionReq{
    fn to_bytes(&self, padding: bool)->Vec<u8> {
        Vec::from(<Self as Pod>::as_bytes(&self))    
    }
}

#[derive(Pod, Clone, Copy)]
#[repr(C)]
pub union VirtioCryptoSymCreateSessionFlf {
    pub CipherFlf : VirtioCryptoCipherSessionFlf,
    pub AlgChainFlf : VirtioCryptoAlgChainSessionFlf
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct VirtioCryptoCipherSessionFlf {
    pub algo: i32,
    pub key_len: i32,
    pub op: i32,
    pub padding: u32
}

impl VirtioCryptoCipherSessionFlf{
    pub fn new(algo: CryptoCipherAlgorithm, key_len: i32, op: CryptoOperation)->Self{
        Self { 
            algo: algo as _, 
            key_len, 
            op: op as _, 
            padding: 0,
        }
    }
}


#[derive(Pod, Clone, Copy)]
#[repr(C)]
pub struct VirtioCryptoAlgChainSessionFlf {
    pub alg_chain_order : i32,
    pub hash_mode : i32,
    pub cipher_hdr : VirtioCryptoCipherSessionFlf,
    pub algo_flf : VirtioCryptoAlgChainSessionAlgo,
    pub aad_len : i32,
    pub padding : i32
}

impl VirtioCryptoAlgChainSessionFlf {
    pub fn new(alg_chain_order: CryptoSymAlgChainOrder, hash_mode: CryptoSymHashMode, cipher_hdr: VirtioCryptoCipherSessionFlf
        , algo_flf: VirtioCryptoAlgChainSessionAlgo, aad_len: i32) -> Self {
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
pub union VirtioCryptoAlgChainSessionAlgo {
    pub hash_flf: VirtioCryptoHashSessionFlf,
    pub mac_flf: VirtioCryptoMacSessionFlf,
    pub padding: [u8; 16]
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
        let header_bytes = self.header.to_bytes(padding);
        let flf_bytes = self.flf.to_bytes(padding);
        return [header_bytes, flf_bytes].concat();
    }
}
