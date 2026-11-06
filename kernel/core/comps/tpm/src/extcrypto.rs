// SPDX-License-Identifier: MPL-2.0

use aes::Aes128;
use cfb_mode::{
    Cfb,
    cipher::{AsyncStreamCipher, NewCipher},
};
use hmac::Mac;
use sha2::Digest;
use tpm_core::crypto::{
    AES_KEY_LEN, AesCfb, CFB_MATERIAL_LEN, HmacSha256Ctx, NONCE_LEN, NonceSource, SHA256_LEN,
    Sha256Ctx,
};

pub(super) struct ExtSha256(sha2::Sha256);

impl Sha256Ctx for ExtSha256 {
    fn new() -> Self {
        Self(sha2::Sha256::new())
    }

    fn update(&mut self, data: &[u8]) {
        self.0.update(data);
    }

    fn finish(self) -> [u8; SHA256_LEN] {
        self.0.finalize().into()
    }
}

pub(super) struct ExtHmacSha256(hmac::Hmac<sha2::Sha256>);

impl HmacSha256Ctx for ExtHmacSha256 {
    fn with_key(key: &[u8]) -> Self {
        Self(hmac::Hmac::new_from_slice(key).expect("HMAC accepts keys of any length"))
    }

    fn update(&mut self, data: &[u8]) {
        self.0.update(data);
    }

    fn finish(self) -> [u8; SHA256_LEN] {
        self.0.finalize().into_bytes().into()
    }
}

pub(super) struct ExtAesCfb;

impl AesCfb for ExtAesCfb {
    fn encrypt(&self, material: &[u8; CFB_MATERIAL_LEN], data: &mut [u8]) {
        cipher(material).encrypt(data);
    }

    fn decrypt(&self, material: &[u8; CFB_MATERIAL_LEN], data: &mut [u8]) {
        cipher(material).decrypt(data);
    }
}

fn cipher(material: &[u8; CFB_MATERIAL_LEN]) -> Cfb<Aes128> {
    Cfb::new_from_slices(&material[..AES_KEY_LEN], &material[AES_KEY_LEN..])
        .expect("AES-CFB key and IV have fixed lengths")
}

pub(super) struct ExtRng<F>(F);

impl<F: FnMut(&mut [u8])> ExtRng<F> {
    pub(super) fn new(fill: F) -> Self {
        Self(fill)
    }
}

impl<F: FnMut(&mut [u8])> NonceSource for ExtRng<F> {
    fn nonce(&mut self) -> [u8; NONCE_LEN] {
        let mut nonce = [0u8; NONCE_LEN];
        (self.0)(&mut nonce);
        nonce
    }
}
