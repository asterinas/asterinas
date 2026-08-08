// SPDX-License-Identifier: MPL-2.0

//! crc32c (Castagnoli) — the checksum kernel behind `metadata_csum` (Phase 6b)
//! and, later, the JBD2 journal checksums (Phase 7).
//!
//! This is the *raw* reflected CRC-32C used by ext4: [`crc32c`] runs the
//! reflected polynomial `0x82F63B78` over the bytes with `seed` as the running
//! state and applies **no** pre- or post-inversion — exactly Linux's
//! `__crc32c_le` and e2fsprogs' `crc32c_le`, which ext4's `ext4_chksum` wraps
//! verbatim. Callers own the conventional `~0` seed (superblock) or the
//! per-filesystem / per-inode seed (group descriptors, inodes, directory and
//! extent blocks); ext4 stores the running value directly, so this function
//! must not invert, or verify-on-read would reject every valid on-disk checksum.
//!
//! The well-known CRC-32C "check" constant `0xE3069283` (init and xorout both
//! `~0`) therefore equals `!crc32c(!0, b"123456789")` here — the outer `!`
//! being the caller-side xorout that ext4 does not use internally.

use super::inode::Ext4Ino;

/// The CRC-32C lookup table, one 32-bit residue per possible input byte,
/// generated at compile time from the reflected polynomial `0x82F63B78`.
const CRC32C_TABLE: [u32; 256] = {
    const POLY: u32 = 0x82F63B78;
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ POLY
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

/// Folds `data` into the running CRC-32C `seed` and returns the new running
/// value (no pre/post inversion — see the module docs).
///
/// Segments chain: `crc32c(crc32c(seed, a), b) == crc32c(seed, a ++ b)`, which
/// is how ext4 checksums a structure across the gap left by its own (zeroed)
/// checksum field.
pub(super) fn crc32c(seed: u32, data: &[u8]) -> u32 {
    let mut crc = seed;
    for &byte in data {
        crc = (crc >> 8) ^ CRC32C_TABLE[((crc ^ byte as u32) & 0xFF) as usize];
    }
    crc
}

/// The per-filesystem metadata_csum seed (`crc32c(!0, uuid)`): feeds the group
/// descriptor and bitmap checksums directly, and folds into a per-inode seed for
/// the inode, directory-block, and extent-node checksums.
#[derive(Clone, Copy, Debug)]
pub(super) struct FsCsumSeed(u32);

impl FsCsumSeed {
    pub(super) fn new(seed: u32) -> Self {
        Self(seed)
    }

    pub(super) fn get(self) -> u32 {
        self.0
    }

    /// Folds `ino` and `generation` into the fs seed (Linux `ext4_inode_csum_seed`).
    pub(super) fn derive_inode(self, ino: Ext4Ino, generation: u32) -> InodeCsumSeed {
        let s = crc32c(self.0, &ino.to_le_bytes());
        InodeCsumSeed(crc32c(s, &generation.to_le_bytes()))
    }
}

/// The per-inode metadata_csum seed `crc32c(crc32c(fs_seed, ino), generation)`.
#[derive(Clone, Copy, Debug)]
pub(super) struct InodeCsumSeed(u32);

impl InodeCsumSeed {
    pub(super) fn get(self) -> u32 {
        self.0
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::*;

    use super::crc32c;

    /// The canonical CRC-32C check vector: `0xE3069283` over `b"123456789"`
    /// with the conventional `~0` init and `~0` xorout. Our raw function omits
    /// the xorout, so the caller applies the outer `!`.
    #[ktest]
    fn crc32c_canonical_check_vector() {
        assert_eq!(!crc32c(!0u32, b"123456789"), 0xE306_9283);
    }

    /// The empty input leaves the running state untouched (the identity of the
    /// fold), so a `~0` seed over nothing xors back to zero.
    #[ktest]
    fn crc32c_empty_is_identity() {
        assert_eq!(crc32c(0, &[]), 0);
        assert_eq!(!crc32c(!0u32, &[]), 0);
    }

    /// Chaining two segments equals one pass over their concatenation — the
    /// property ext4 relies on to checksum a struct around its checksum field.
    #[ktest]
    fn crc32c_segments_chain() {
        let whole = crc32c(!0u32, b"123456789");
        let split = crc32c(crc32c(!0u32, b"12345"), b"6789");
        assert_eq!(whole, split);
    }

    /// A single-byte change flips the result (guards against a stuck table).
    #[ktest]
    fn crc32c_detects_single_bit() {
        let a = crc32c(!0u32, b"metadata_csum");
        let b = crc32c(!0u32, b"metadata_csun");
        assert_ne!(a, b);
    }
}
