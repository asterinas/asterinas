// SPDX-License-Identifier: MPL-2.0

use core::ops::Deref;

use crate::prelude::Result;

/// Random initialization for Key, Iv and Mac.
pub trait RandomInit: Default {
    fn random() -> Self;
}

/// Authenticated Encryption with Associated Data (AEAD) algorithm.
pub trait Aead {
    type Key: Deref<Target = [u8]> + RandomInit;
    type Iv: Deref<Target = [u8]> + RandomInit;
    type Mac: Deref<Target = [u8]> + RandomInit;

    /// Encrypt plaintext referred by `input`, with a secret `Key`,
    /// initialization vector `Iv` and additional associated data `aad`.
    ///
    /// If the operation succeed, the ciphertext will be written to `output`
    /// and a message authentication code `Mac` will be returned. Or else,
    /// return an `Error` on any fault.
    fn encrypt(
        &self,
        input: &[u8],
        key: &Self::Key,
        iv: &Self::Iv,
        aad: &[u8],
        output: &mut [u8],
    ) -> Result<Self::Mac>;

    /// Decrypt ciphertext referred by `input`, with a secret `Key` and
    /// message authentication code `Mac`, initialization vector `Iv` and
    /// additional associated data `aad`.
    ///
    /// If the operation succeed, the plaintext will be written to `output`.
    /// Or else, return an `Error` on any fault.
    fn decrypt(
        &self,
        input: &[u8],
        key: &Self::Key,
        iv: &Self::Iv,
        aad: &[u8],
        mac: &Self::Mac,
        output: &mut [u8],
    ) -> Result<()>;
}

/// Symmetric key cipher algorithm.
pub trait Skcipher {
    type Key: Deref<Target = [u8]> + RandomInit;
    type Iv: Deref<Target = [u8]> + RandomInit;

    /// Encrypt plaintext referred by `input`, with a secret `Key` and
    /// initialization vector `Iv`.
    ///
    /// If the operation succeed, the ciphertext will be written to `output`.
    /// Or else, return an `Error` on any fault.
    fn encrypt(
        &self,
        input: &[u8],
        key: &Self::Key,
        iv: &Self::Iv,
        output: &mut [u8],
    ) -> Result<()>;

    /// Decrypt ciphertext referred by `input` with a secret `Key` and
    /// initialization vector `Iv`.
    ///
    /// If the operation succeed, the plaintext will be written to `output`.
    /// Or else, return an `Error` on any fault.
    fn decrypt(
        &self,
        input: &[u8],
        key: &Self::Key,
        iv: &Self::Iv,
        output: &mut [u8],
    ) -> Result<()>;
}

/// Random number generator.
pub trait Rng {
    /// Create an instance, with `seed` to provide secure entropy.
    fn new(seed: &[u8]) -> Self;

    /// Fill `dest` with random bytes.
    fn fill_bytes(&self, dest: &mut [u8]) -> Result<()>;
}
