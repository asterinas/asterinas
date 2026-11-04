// SPDX-License-Identifier: MPL-2.0

use tpm_core::crypto::{
    AES_BLOCK_LEN, AES_KEY_LEN, AesCfb, CFB_MATERIAL_LEN, HmacSha256Ctx, NONCE_LEN, NonceSource,
    SHA256_LEN, Sha256Ctx,
};

/// Legacy context capacity retained for API compatibility.
pub const HASH_CTX_CAP: usize = 512;
// Rust crypto contexts need no ABI size check, but the boot API requires this hook.
pub fn check_abi() -> bool {
    true
}
struct Sha256State {
    inner: sha2::Sha256,
}
pub struct ExtSha256 {
    st: Sha256State,
}
impl Sha256Ctx for ExtSha256 {
    fn new() -> Self {
        use sha2::Digest;
        ExtSha256 {
            st: Sha256State {
                inner: sha2::Sha256::new(),
            },
        }
    }
    fn update(&mut self, data: &[u8]) {
        use sha2::Digest;
        self.st.inner.update(data);
    }
    fn finish(self) -> [u8; SHA256_LEN] {
        use sha2::Digest;
        let me = self;
        let digest = me.st.inner.finalize();
        let mut out = [0u8; SHA256_LEN];
        out.copy_from_slice(digest.as_ref());
        out
    }
}
struct HmacState {
    inner: hmac::Hmac<sha2::Sha256>,
}
pub struct ExtHmacSha256 {
    st: HmacState,
}
impl HmacSha256Ctx for ExtHmacSha256 {
    fn with_key(key: &[u8]) -> Self {
        use hmac::Mac;
        let inner = hmac::Hmac::<sha2::Sha256>::new_from_slice(key).expect("HMAC 接受任意长度密钥");
        ExtHmacSha256 {
            st: HmacState { inner },
        }
    }
    fn update(&mut self, data: &[u8]) {
        use hmac::Mac;
        self.st.inner.update(data);
    }
    fn finish(self) -> [u8; SHA256_LEN] {
        use hmac::Mac;
        let me = self;
        let tag = me.st.inner.finalize().into_bytes();
        let mut out = [0u8; SHA256_LEN];
        out.copy_from_slice(tag.as_ref());
        out
    }
}
pub struct ExtAesCfb;
impl AesCfb for ExtAesCfb {
    fn encrypt(&self, material: &[u8; CFB_MATERIAL_LEN], data: &mut [u8]) {
        cfb_apply(material, data, true);
    }
    fn decrypt(&self, material: &[u8; CFB_MATERIAL_LEN], data: &mut [u8]) {
        cfb_apply(material, data, false);
    }
}

// Cipher setup failures must abort rather than risk sending plaintext.
#[inline]
fn cfb_apply(material: &[u8; CFB_MATERIAL_LEN], data: &mut [u8], encrypt: bool) {
    use aes::Aes128;
    use cfb_mode::{
        Cfb,
        cipher::{AsyncStreamCipher, NewCipher},
    };

    const {
        debug_assert!(AES_KEY_LEN + AES_BLOCK_LEN == CFB_MATERIAL_LEN);
    }
    let mut key = [0u8; AES_KEY_LEN];
    let mut iv = [0u8; AES_BLOCK_LEN];
    key.copy_from_slice(&material[..AES_KEY_LEN]);
    iv.copy_from_slice(&material[AES_KEY_LEN..]);

    if encrypt {
        Cfb::<Aes128>::new_from_slices(&key, &iv)
            .expect("密钥与 IV 长度固定为 16 字节")
            .encrypt(data);
    } else {
        Cfb::<Aes128>::new_from_slices(&key, &iv)
            .expect("密钥与 IV 长度固定为 16 字节")
            .decrypt(data);
    }

    wipe_bytes(&mut key);
    wipe_bytes(&mut iv);
}

#[inline]
fn wipe_bytes(b: &mut [u8]) {
    b.fill(0);
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
}

const RNG_RETRY: u32 = 16;
pub struct ExtRng;
impl ExtRng {
    pub fn new() -> Self {
        Self
    }
}
impl Default for ExtRng {
    fn default() -> Self {
        Self::new()
    }
}
impl NonceSource for ExtRng {
    // Nonces must stay unpredictable; abort if entropy remains unavailable.
    fn nonce(&mut self) -> [u8; NONCE_LEN] {
        let mut out = [0u8; NONCE_LEN];
        let mut offset = 0usize;
        let mut tries = 0u32;
        while offset < NONCE_LEN {
            if let Some(value) = ostd::arch::read_random() {
                let bytes = value.to_ne_bytes();
                let count = core::cmp::min(bytes.len(), NONCE_LEN - offset);
                out[offset..offset + count].copy_from_slice(&bytes[..count]);
                offset += count;
                tries = 0;
            } else {
                tries += 1;
                if tries >= RNG_RETRY {
                    ostd::panic::abort();
                }
            }
        }
        out
    }
}
