// SPDX-License-Identifier: MPL-2.0

//! A `no_std` BLAKE2s-256 implementation.
//!
//! For the specification, see RFC 7693: <https://www.rfc-editor.org/rfc/rfc7693.html>.
//!
//! This crate provides unkeyed and keyed BLAKE2s-256 hashing with a small `no_std` streaming API.

#![no_std]
#![deny(unsafe_code)]

pub const BLAKE2S_BLOCK_SIZE: usize = 64;
pub const BLAKE2S_HASH_SIZE: usize = 32;
pub const BLAKE2S_KEY_SIZE: usize = 32;

const IV: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

const SIGMA: [[usize; 16]; 10] = [
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
    [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
    [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
    [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
    [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
    [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
    [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
    [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
    [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
];

/// An error returned by BLAKE2s constructors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Blake2sError {
    /// The key is longer than `BLAKE2S_KEY_SIZE`.
    KeyTooLong,
}

pub struct Blake2s {
    h: [u32; 8],
    t: [u32; 2],
    f: [u32; 2],
    buf: [u8; BLAKE2S_BLOCK_SIZE],
    buflen: usize,
}

impl Blake2s {
    /// Creates a new unkeyed BLAKE2s-256 context.
    pub fn new() -> Self {
        Self::new_with_key_length(0)
    }

    /// Creates a new keyed BLAKE2s-256 context.
    pub fn new_keyed(key: &[u8]) -> Result<Self, Blake2sError> {
        if key.len() > BLAKE2S_KEY_SIZE {
            return Err(Blake2sError::KeyTooLong);
        }

        let mut context = Self::new_with_key_length(key.len());
        if !key.is_empty() {
            let mut block = [0; BLAKE2S_BLOCK_SIZE];
            block[..key.len()].copy_from_slice(key);
            context.update(&block);
            block.fill(0);
        }

        Ok(context)
    }

    /// Updates the context with `input`.
    pub fn update(&mut self, mut input: &[u8]) {
        if input.is_empty() {
            return;
        }

        let fill = BLAKE2S_BLOCK_SIZE - self.buflen;
        if input.len() > fill {
            self.buf[self.buflen..][..fill].copy_from_slice(&input[..fill]);
            self.compress_buffer(BLAKE2S_BLOCK_SIZE as u32);
            self.buflen = 0;
            input = &input[fill..];
        }

        while input.len() > BLAKE2S_BLOCK_SIZE {
            let mut block = [0; BLAKE2S_BLOCK_SIZE];
            block.copy_from_slice(&input[..BLAKE2S_BLOCK_SIZE]);
            self.compress(&block, BLAKE2S_BLOCK_SIZE as u32);
            input = &input[BLAKE2S_BLOCK_SIZE..];
        }

        self.buf[self.buflen..][..input.len()].copy_from_slice(input);
        self.buflen += input.len();
    }

    /// Finalizes the context and returns the BLAKE2s-256 digest.
    pub fn finalize(mut self) -> [u8; BLAKE2S_HASH_SIZE] {
        self.set_lastblock();
        self.buf[self.buflen..].fill(0);
        self.compress_buffer(self.buflen as u32);

        let mut output = [0; BLAKE2S_HASH_SIZE];
        for (chunk, word) in output.chunks_exact_mut(size_of::<u32>()).zip(self.h.iter()) {
            chunk.copy_from_slice(&word.to_le_bytes());
        }

        self.clear();
        output
    }

    fn new_with_key_length(key_len: usize) -> Self {
        let mut context = Self {
            h: IV,
            t: [0; 2],
            f: [0; 2],
            buf: [0; BLAKE2S_BLOCK_SIZE],
            buflen: 0,
        };
        context.h[0] ^= 0x0101_0000 ^ ((key_len as u32) << 8) ^ (BLAKE2S_HASH_SIZE as u32);
        context
    }

    fn increment_counter(&mut self, inc: u32) {
        let (counter, overflow) = self.t[0].overflowing_add(inc);
        self.t[0] = counter;
        self.t[1] = self.t[1].wrapping_add(u32::from(overflow));
    }

    fn set_lastblock(&mut self) {
        self.f[0] = u32::MAX;
    }

    fn compress_buffer(&mut self, inc: u32) {
        let block = self.buf;
        self.compress(&block, inc);
    }

    fn compress(&mut self, block: &[u8; BLAKE2S_BLOCK_SIZE], inc: u32) {
        self.increment_counter(inc);

        let mut message = [0; 16];
        for (word, chunk) in message.iter_mut().zip(block.chunks_exact(size_of::<u32>())) {
            let mut bytes = [0; size_of::<u32>()];
            bytes.copy_from_slice(chunk);
            *word = u32::from_le_bytes(bytes);
        }

        let mut work = [0; 16];
        work[..8].copy_from_slice(&self.h);
        work[8..].copy_from_slice(&IV);
        work[12] ^= self.t[0];
        work[13] ^= self.t[1];
        work[14] ^= self.f[0];
        work[15] ^= self.f[1];

        for schedule in SIGMA {
            Self::g(&mut work, &message, schedule, 0, 4, 8, 12, 0);
            Self::g(&mut work, &message, schedule, 1, 5, 9, 13, 1);
            Self::g(&mut work, &message, schedule, 2, 6, 10, 14, 2);
            Self::g(&mut work, &message, schedule, 3, 7, 11, 15, 3);
            Self::g(&mut work, &message, schedule, 0, 5, 10, 15, 4);
            Self::g(&mut work, &message, schedule, 1, 6, 11, 12, 5);
            Self::g(&mut work, &message, schedule, 2, 7, 8, 13, 6);
            Self::g(&mut work, &message, schedule, 3, 4, 9, 14, 7);
        }

        for index in 0..8 {
            self.h[index] ^= work[index] ^ work[index + 8];
        }
    }

    #[expect(clippy::too_many_arguments)]
    fn g(
        work: &mut [u32; 16],
        message: &[u32; 16],
        schedule: [usize; 16],
        a: usize,
        b: usize,
        c: usize,
        d: usize,
        index: usize,
    ) {
        work[a] = work[a]
            .wrapping_add(work[b])
            .wrapping_add(message[schedule[2 * index]]);
        work[d] = (work[d] ^ work[a]).rotate_right(16);
        work[c] = work[c].wrapping_add(work[d]);
        work[b] = (work[b] ^ work[c]).rotate_right(12);
        work[a] = work[a]
            .wrapping_add(work[b])
            .wrapping_add(message[schedule[2 * index + 1]]);
        work[d] = (work[d] ^ work[a]).rotate_right(8);
        work[c] = work[c].wrapping_add(work[d]);
        work[b] = (work[b] ^ work[c]).rotate_right(7);
    }

    fn clear(&mut self) {
        self.h.fill(0);
        self.t.fill(0);
        self.f.fill(0);
        self.buf.fill(0);
        self.buflen = 0;
    }
}

impl Default for Blake2s {
    fn default() -> Self {
        Self::new()
    }
}

/// Computes the BLAKE2s-256 digest of `input`.
pub fn blake2s(input: &[u8]) -> [u8; BLAKE2S_HASH_SIZE] {
    let mut context = Blake2s::new();
    context.update(input);
    context.finalize()
}

/// Computes the keyed BLAKE2s-256 digest of `input`.
pub fn blake2s_keyed(key: &[u8], input: &[u8]) -> Result<[u8; BLAKE2S_HASH_SIZE], Blake2sError> {
    let mut context = Blake2s::new_keyed(key)?;
    context.update(input);
    Ok(context.finalize())
}

#[cfg(test)]
mod test {
    use super::*;

    const EMPTY_HASH: [u8; BLAKE2S_HASH_SIZE] = [
        0x69, 0x21, 0x7a, 0x30, 0x79, 0x90, 0x80, 0x94, 0xe1, 0x11, 0x21, 0xd0, 0x42, 0x35, 0x4a,
        0x7c, 0x1f, 0x55, 0xb6, 0x48, 0x2c, 0xa1, 0xa5, 0x1e, 0x1b, 0x25, 0x0d, 0xfd, 0x1e, 0xd0,
        0xee, 0xf9,
    ];

    const ABC_HASH: [u8; BLAKE2S_HASH_SIZE] = [
        0x50, 0x8c, 0x5e, 0x8c, 0x32, 0x7c, 0x14, 0xe2, 0xe1, 0xa7, 0x2b, 0xa3, 0x4e, 0xeb, 0x45,
        0x2f, 0x37, 0x45, 0x8b, 0x20, 0x9e, 0xd6, 0x3a, 0x29, 0x4d, 0x99, 0x9b, 0x4c, 0x86, 0x67,
        0x59, 0x82,
    ];

    const RANGE_HASH: [u8; BLAKE2S_HASH_SIZE] = [
        0x5f, 0xde, 0xb5, 0x9f, 0x68, 0x1d, 0x97, 0x5f, 0x52, 0xc8, 0xe6, 0x9c, 0x55, 0x02, 0xe0,
        0x2a, 0x12, 0xa3, 0xaf, 0xcc, 0x58, 0x36, 0xba, 0x58, 0xf4, 0x27, 0x84, 0xc4, 0x39, 0x22,
        0x87, 0x81,
    ];

    const KEYED_ABC_HASH: [u8; BLAKE2S_HASH_SIZE] = [
        0xa2, 0x81, 0xf7, 0x25, 0x75, 0x49, 0x69, 0xa7, 0x02, 0xf6, 0xfe, 0x36, 0xfc, 0x59, 0x1b,
        0x7d, 0xef, 0x86, 0x6e, 0x4b, 0x70, 0x17, 0x3e, 0xce, 0x40, 0x2f, 0xc0, 0x1c, 0x06, 0x4d,
        0x6b, 0x65,
    ];

    #[test]
    fn empty_input_matches_test_vector() {
        assert_eq!(blake2s(b""), EMPTY_HASH);
    }

    #[test]
    fn short_input_matches_test_vector() {
        assert_eq!(blake2s(b"abc"), ABC_HASH);
    }

    #[test]
    fn multi_block_input_matches_test_vector() {
        let mut input = [0; 256];
        for (index, byte) in input.iter_mut().enumerate() {
            *byte = index as u8;
        }

        assert_eq!(blake2s(&input), RANGE_HASH);
    }

    #[test]
    fn keyed_hash_matches_test_vector() {
        let mut key = [0; BLAKE2S_KEY_SIZE];
        for (index, byte) in key.iter_mut().enumerate() {
            *byte = index as u8;
        }

        assert_eq!(blake2s_keyed(&key, b"abc").unwrap(), KEYED_ABC_HASH);
    }

    #[test]
    fn incremental_update_matches_one_shot() {
        let input = b"the quick brown fox jumps over the lazy dog";
        let mut context = Blake2s::new();
        context.update(&input[..3]);
        context.update(&input[3..17]);
        context.update(&input[17..]);

        assert_eq!(context.finalize(), blake2s(input));
    }

    #[test]
    fn rejects_long_keys() {
        let key = [0; BLAKE2S_KEY_SIZE + 1];

        assert!(matches!(
            Blake2s::new_keyed(&key),
            Err(Blake2sError::KeyTooLong)
        ));
    }
}
