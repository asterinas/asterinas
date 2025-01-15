use core::hash;

use alloc::vec::Vec;
use aster_crypto::{CryptoCipherAlgorithm, CryptoError, CryptoHashAlgorithm, CryptoOperation};
use ostd::Pod;
use crate::device::crypto::session::*;

#[derive(Debug, Clone, Copy)]
#[repr(i32)]
pub enum CryptoServiceOperation{
    CipherEncrypt = crypto_services_opcode(CryptoService::Cipher, 0x00),
    CipherDecrypt = crypto_services_opcode(CryptoService::Cipher, 0x01),
    Hash = crypto_services_opcode(CryptoService::Hash, 0x00),
    Mac = crypto_services_opcode(CryptoService::Mac, 0x00),
    AeadEncrypt = crypto_services_opcode(CryptoService::Aead, 0x00),
    AeadDecrypt = crypto_services_opcode(CryptoService::Aead, 0x01),
    AkCipherEncrypt = crypto_services_opcode(CryptoService::AkCipher, 0x00),
    AkCipherDecrypt = crypto_services_opcode(CryptoService::AkCipher, 0x01),
    AkCipherSign = crypto_services_opcode(CryptoService::AkCipher, 0x02),
    AkCipherVerify = crypto_services_opcode(CryptoService::AkCipher, 0x03),
}
#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoServiceHeader {
    opcode : i32,
    algo : i32,
    session_id : i64,
    flag : i32,
    padding : i32
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoSeriviceResp {
    status : u8
}


#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoCipherServiceReq {
    header : CryptoServiceHeader,
    op_flf : VirtioCryptoSymDataFlf,
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct VirtioCryptoSymDataFlf {
    op_type_flf : [u8 ; 40],
    op_type : i32,
    padding : i32
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct VirtioCryptoCipherDataFlf {
    iv_len : i32,
    src_data_len : i32,
    dst_data_len : i32,
    padding : i32
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct VirtioCryptoAlgChainDataFlf {
    iv_len : i32,
    src_data_len : i32,
    dst_data_len : i32,
    cipher_start_src_offset : i32,
    len_to_cipher : i32,
    hash_start_src_offset : i32,
    len_to_hash : i32,
    aad_len : i32,
    hash_result_len : i32,
    reserved : i32
}

pub struct VirtioCryptoSymDataVlf {
    op_type_vlf : Vec<u8>
}

pub struct VirtioCryptoCipherDataVlf {
    iv : Vec<u8>,
    src_data : Vec<u8>,
    dst_data : Vec<u8>
}

pub struct VirtioCryptoAlgChainDataVlf {
    iv : Vec<u8>,
    src_data : Vec<u8>,
    aad : Vec<u8>,
    dst_data : Vec<u8>,
    hash_result : Vec<u8>
}



