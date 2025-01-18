// // SPDX-License-Identifier: MPL-2.0

// use alloc::vec::Vec;
// use aster_crypto::*;
// use ostd::Pod;

// pub enum VirtioCryptoStatus { 
//     Ok = 0,             // success
//     Err = 1,            // any failure not mentioned above occurs
//     BadMsg = 2,         // authentication failed (only when AEAD decryption)
//     NotSupp = 3,        // operation or algorithm is unsupported
//     InvSess = 4,        // invalid session ID when executing crypto operations
//     NoSpc = 5,          // no free session ID.
//     KeyReject = 6,      // signature verification failed (only when AKCIPHER verification)
// }

// impl TryFrom<i32> for VirtioCryptoStatus {
//     type Error = CryptoError;

//     fn try_from(value: i32) -> Result<Self, Self::Error> {
//         match value {
//             0 => Ok(Self::Ok),
//             1 => Ok(Self::Err),
//             2 => Ok(Self::BadMsg),
//             3 => Ok(Self::NotSupp),
//             4 => Ok(Self::InvSess),
//             5 => Ok(Self::NoSpc),
//             6 => Ok(Self::KeyReject),
//             _ => Err(CryptoError::UnknownError),
//         }
//     }
// }

// impl VirtioCryptoStatus{
//     pub fn get_or_error<T>(&self, val: T)->Result<T, CryptoError>{
//         match self {
//             VirtioCryptoStatus::Ok => Ok(val),
//             VirtioCryptoStatus::Err => Err(CryptoError::UnknownError),
//             VirtioCryptoStatus::BadMsg => Err(CryptoError::BadMessage),
//             VirtioCryptoStatus::NotSupp => Err(CryptoError::NotSupport),
//             VirtioCryptoStatus::InvSess => Err(CryptoError::InvalidSession),
//             VirtioCryptoStatus::NoSpc => Err(CryptoError::NoFreeSession),
//             VirtioCryptoStatus::KeyReject => Err(CryptoError::KeyReject)
//         }
//     }
// }

// #[derive(Debug, Pod, Clone, Copy)]
// #[repr(C)]
// pub struct CryptoCtrlHeader{
//     pub opcode: i32,
//     pub algo: i32,
//     pub flag: i32,
//     pub reserved: i32,
// }

// impl CryptoCtrlHeader {
//     pub fn to_bytes(&self) -> Vec<u8> {
//         <Self as Pod>::as_bytes(&self).to_vec()
//     }
// }

// pub struct CryptoSessionRequest<T>{
//     pub header: CryptoCtrlHeader,
//     pub flf: T
// }

// impl<T: AutoPadding> CryptoSessionRequest<T>{
//     pub fn to_bytes(&self, revision_1: bool)->Vec<u8>{
//         let header_bytes = self.header.to_bytes();
//         let flf_bytes = self.flf.to_bytes(revision_1);
//         return [header_bytes, flf_bytes].concat();   
//     }

//     pub fn len(&self)->usize{
//         return size_of::<CryptoCtrlHeader>() + size_of::<T>();
//     }
// }

// pub trait AutoPadding: Pod{
//     const PADDING_BYTES: usize;

//     fn to_bytes(&self, revision_1: bool)->Vec<u8>{
//         let res = <Self as Pod>::as_bytes(&self);
//         let mut vec = Vec::from(res);
//         if revision_1 {
//             vec.resize(Self::PADDING_BYTES, 0);
//         }
//         vec        
//     }
// }

// #[derive(Debug, Pod, Clone, Copy)]
// #[repr(C)]
// pub struct CryptoHashSessionFlf {
//     pub algo: i32,
//     pub hash_result_len: u32
// }

// impl AutoPadding for CryptoHashSessionFlf{
//     const PADDING_BYTES: usize = 56;
// }

// #[derive(Pod, Clone, Copy)]
// #[repr(C)]
// pub struct CryptoAeadSessionFlf{
//     pub algo : i32,
//     pub key_len : i32,
//     pub tag_len : i32,
//     pub aad_len : i32,
//     pub op : i32,
//     pub padding : i32
// }

// impl AutoPadding for CryptoAeadSessionFlf{
//     const PADDING_BYTES: usize = 56;
// }

// #[derive(Debug, Pod, Clone, Copy)]
// #[repr(C)]
// pub struct CryptoMacSessionFlf{
//     pub algo: i32,
//     pub mac_result_len: u32,
//     pub auth_key_len: u32,
//     pub padding: i32,
// }

// impl AutoPadding for CryptoMacSessionFlf{
//     const PADDING_BYTES: usize = 56;
// }

// #[derive(Debug, Pod, Clone, Copy)]
// #[repr(C)]
// pub struct CryptoDestroySessionFlf {
//     pub session_id : i64
// }

// impl AutoPadding for CryptoDestroySessionFlf{
//     const PADDING_BYTES: usize = 56;
// }

// #[derive(Pod, Clone, Copy)]
// #[repr(C)]
// pub union CryptoSymSessionOpFlf {
//     pub cipher_flf : CryptoCipherSessionFlf,
//     pub alg_chain_flf : CryptoAlgChainSessionFlf
// }

// #[derive(Pod, Clone, Copy)]
// #[repr(C)]
// pub struct CryptoSymCreateSessionFlf{
//     pub op_flf: CryptoSymSessionOpFlf,
//     pub op_type: i32,
//     pub padding: i32
// }

// impl AutoPadding for CryptoSymCreateSessionFlf{
//     const PADDING_BYTES: usize = 56;
// }

// #[derive(Debug, Pod, Clone, Copy)]
// #[repr(C)]
// pub struct CryptoRSAPara {
//     pub padding_algo: i32,
//     pub hash_algo: i32,
// }

// #[derive(Debug, Pod, Clone, Copy)]
// #[repr(C)]
// pub struct CryptoECDSAPara {
//     pub curve_id: i32,
// }


// #[derive(Pod, Clone, Copy)]
// #[repr(C)]
// pub union CryptoAkCipherAlgoFlf {
//     pub rsa: CryptoRSAPara,
//     pub ecdsa: CryptoECDSAPara,
// }

// #[derive(Pod, Clone, Copy)]
// #[repr(C)]
// pub struct CryptoAkCipherSessionFlf {
//     pub algo: i32,
//     pub key_type: i32,
//     pub key_len: u32,
//     pub algo_flf: CryptoAkCipherAlgoFlf,
// }

// impl AutoPadding for CryptoAkCipherSessionFlf{
//     const PADDING_BYTES: usize = 56;
// }

// #[derive(Debug, Pod, Clone, Copy)]
// #[repr(C)]
// pub struct CryptoCipherSessionFlf {
//     pub algo: i32,
//     pub key_len: i32,
//     pub op: i32,
//     pub padding: u32
// }

// #[derive(Pod, Clone, Copy)]
// #[repr(C)]
// pub union CryptoAlgChainSessionAlgo {
//     pub hash_flf: CryptoHashSessionFlf,
//     pub mac_flf: CryptoMacSessionFlf,
//     pub padding: [u8; 16]
// }

// #[derive(Pod, Clone, Copy)]
// #[repr(C)]
// pub struct CryptoAlgChainSessionFlf {
//     pub alg_chain_order : i32,
//     pub hash_mode : i32,
//     pub cipher_hdr : CryptoCipherSessionFlf,
//     pub algo_flf : CryptoAlgChainSessionAlgo,
//     pub aad_len : i32,
//     pub padding : i32
// }

// impl CryptoAlgChainSessionFlf {
//     pub fn new(alg_chain_order: CryptoSymAlgChainOrder, hash_mode: CryptoSymHashMode, cipher_hdr: CryptoCipherSessionFlf,
//                algo_flf: CryptoAlgChainSessionAlgo, aad_len: i32) -> Self {
//             Self {
//                 alg_chain_order: alg_chain_order as _,
//                 hash_mode: hash_mode as _,
//                 cipher_hdr,
//                 algo_flf,
//                 aad_len,
//                 padding: 0
//             }
//         }
// }

// #[derive(Debug, Pod, Clone, Copy)]
// #[repr(C)]
// pub struct VirtioCryptoSessionInput{
//     pub session_id: i64,
//     pub status: i32,
//     pub padding: i32,
// }

// impl VirtioCryptoSessionInput{
//     pub fn get_result(&self)->Result<i64, CryptoError>{
//         match VirtioCryptoStatus::try_from(self.status){
//             Ok(code) => code.get_or_error(self.session_id),
//             Err(err) => Err(err)
//         }
//     }
// }

// #[derive(Debug, Pod, Clone, Copy)]
// #[repr(C)]
// pub struct VirtioCryptoDestroySessionInput {
//     pub status : u8
// }

// impl VirtioCryptoDestroySessionInput {
//     pub fn get_result(&self) -> Result<u8, CryptoError> {
//         match VirtioCryptoStatus::try_from(self.status as i32){
//             Ok(code) => code.get_or_error(self.status),
//             Err(err) => Err(err)
//         }
//     }
// }