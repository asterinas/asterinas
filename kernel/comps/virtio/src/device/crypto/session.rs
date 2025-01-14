
use ostd::Pod;

enum CryptoService{
    Cipher = 0,
    Hash = 1,
    Mac = 2,
    Aead = 3,
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

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct CryptoHashSessionReq {
	pub header: CryptoCtrlHeader,
	pub flf: VirtioCryptoHashCreateSessionReq,
    pub padding: [i32; 12]
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct VirtioCryptoSessionInput{
    pub session_id: i64,
    pub status: i32,
    pub padding: i32,
}

#[derive(Debug, Clone, Copy)]
#[repr(i32)]
pub enum CryptoHashAlgorithm {
    NoHash = 0,
    Md5 = 1,
    Sha1 = 2,
    Sha224 = 3,
    Sha256 = 4,
    Sha384 = 5,
    Sha512 = 6,
    Sha3_224 = 7,
    Sha3_256 = 8,
    Sha3_384 = 9,
    Sha3_512 = 10,
    Sha3Shake128 = 11,
    Sha3Shake256 = 12,
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct VirtioCryptoHashSessionPara {
    pub algo: i32,
    pub hash_result_len: u32,
}

#[derive(Debug, Pod, Clone, Copy)]
#[repr(C)]
pub struct VirtioCryptoHashCreateSessionReq {
    pub para: VirtioCryptoHashSessionPara,
}

impl VirtioCryptoHashCreateSessionReq{
    pub fn new(algo: CryptoHashAlgorithm, result_len: u32)->Self{
        Self { 
            para: VirtioCryptoHashSessionPara{
                algo: algo as _,
                hash_result_len: result_len
            } 
        }
    }
}