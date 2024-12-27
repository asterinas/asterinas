// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use lending_iterator::prelude::*;
use ostd_pod::Pod;

use super::{Iv, Key, Mac};
use crate::{
    layers::bio::{BlockId, BlockLog, Buf, BLOCK_SIZE},
    os::Aead,
    prelude::*,
};

/// A cryptographically-protected chain of blocks.
///
/// `CryptoChain<L>` allows writing and reading a sequence of
/// consecutive blocks securely to and from an untrusted storage of data log
/// `L: BlockLog`.
/// The target use case of `CryptoChain` is to implement secure journals,
/// where old data are scanned and new data are appended.
///
/// # On-disk format
///
/// The on-disk format of each block is shown below.
///
/// ```text
/// ┌─────────────────────┬───────┬──────────┬──────────┬──────────┬─────────┐
/// │  Encrypted payload  │  Gap  │  Length  │  PreMac  │  CurrMac │   IV    │
/// │(Length <= 4KB - 48B)│       │   (4B)   │  (16B)   │   (16B)  │  (12B)  │
/// └─────────────────────┴───────┴──────────┴──────────┴──────────┴─────────┘
///
/// ◄─────────────────────────── Block size (4KB) ──────────────────────────►
/// ```
///
/// Each block begins with encrypted user payload. The size of payload
/// must be smaller than that of block size as each block ends with a footer
/// (in plaintext).
/// The footer consists of fours parts: the length of the payload (in bytes),
/// the MAC of the previous block, the MAC of the current block, the IV used
/// for encrypting the current block.
/// The MAC of a block protects the encrypted payload, its length, and the MAC
/// of the previous block.
///
/// # Security
///
/// Each `CryptoChain` is assigned a randomly-generated encryption key.
/// Each block is encrypted using this key and a randomly-generated IV.
/// This setup ensures the confidentiality of payload and even the same payloads
/// result in different ciphertexts.
///
/// `CryptoChain` is called a "chain" of blocks because each block
/// not only stores its own MAC, but also the MAC of its previous block.
/// This effectively forms a "chain" (much like a blockchain),
/// ensuring the orderness and consecutiveness of the sequence of blocks.
///
/// Due to this chain structure, the integrity of a `CryptoChain` can be ensured
/// by verifying the MAC of the last block. Once the integrity of the last block
/// is verified, the integrity of all previous blocks can also be verified.
pub struct CryptoChain<L> {
    block_log: L,
    key: Key,
    block_range: Range<BlockId>,
    block_macs: Vec<Mac>,
}

#[repr(C)]
#[derive(Copy, Clone, Pod)]
struct Footer {
    len: u32,
    pre_mac: Mac,
    this_mac: Mac,
    this_iv: Iv,
}

impl<L: BlockLog> CryptoChain<L> {
    /// The available size in each chained block is smaller than that of
    /// the block size.
    pub const AVAIL_BLOCK_SIZE: usize = BLOCK_SIZE - core::mem::size_of::<Footer>();

    /// Construct a new `CryptoChain` using `block_log: L` as the storage.
    pub fn new(block_log: L) -> Self {
        Self {
            block_log,
            block_range: 0..0,
            key: Key::random(),
            block_macs: Vec::new(),
        }
    }

    /// Recover an existing `CryptoChain` backed by `block_log: L`,
    /// starting from its `from` block.
    pub fn recover(key: Key, block_log: L, from: BlockId) -> Recovery<L> {
        Recovery::new(block_log, key, from)
    }

    /// Read a block at a specified position.
    ///
    /// The length of the given buffer should not be smaller than payload_len
    /// stored in `Footer`.
    ///
    /// # Security
    ///
    /// The authenticity of the block is guaranteed.
    pub fn read(&self, pos: BlockId, buf: &mut [u8]) -> Result<usize> {
        if !self.block_range().contains(&pos) {
            return_errno_with_msg!(NotFound, "read position is out of range");
        }

        // Read block and get footer.
        let mut block_buf = Buf::alloc(1)?;
        self.block_log.read(pos, block_buf.as_mut())?;
        let footer: Footer = Pod::from_bytes(&block_buf.as_slice()[Self::AVAIL_BLOCK_SIZE..]);

        let payload_len = footer.len as usize;
        if payload_len > Self::AVAIL_BLOCK_SIZE || payload_len > buf.len() {
            return_errno_with_msg!(OutOfDisk, "wrong payload_len or the read_buf is too small");
        }

        // Check the footer MAC, to ensure the orderness and consecutiveness of blocks.
        let this_mac = self.block_macs.get(pos - self.block_range.start).unwrap();
        if footer.this_mac.as_bytes() != this_mac.as_bytes() {
            return_errno_with_msg!(NotFound, "check footer MAC failed");
        }

        // Decrypt payload.
        let aead = Aead::new();
        aead.decrypt(
            &block_buf.as_slice()[..payload_len],
            self.key(),
            &footer.this_iv,
            &footer.pre_mac,
            &footer.this_mac,
            &mut buf[..payload_len],
        )?;
        Ok(payload_len)
    }

    /// Append a block at the end.
    ///
    /// The length of the given buffer must not be larger than `AVAIL_BLOCK_SIZE`.
    ///
    /// # Security
    ///
    /// The confidentiality of the block is guaranteed.
    pub fn append(&mut self, buf: &[u8]) -> Result<()> {
        if buf.len() > Self::AVAIL_BLOCK_SIZE {
            return_errno_with_msg!(OutOfDisk, "append data is too large");
        }
        let mut block_buf = Buf::alloc(1)?;

        // Encrypt payload.
        let aead = Aead::new();
        let this_iv = Iv::random();
        let pre_mac = self.block_macs.last().copied().unwrap_or_default();
        let output = &mut block_buf.as_mut_slice()[..buf.len()];
        let this_mac = aead.encrypt(buf, self.key(), &this_iv, &pre_mac, output)?;

        // Store footer.
        let footer = Footer {
            len: buf.len() as _,
            pre_mac,
            this_mac,
            this_iv,
        };
        let buf = &mut block_buf.as_mut_slice()[Self::AVAIL_BLOCK_SIZE..];
        buf.copy_from_slice(footer.as_bytes());

        self.block_log.append(block_buf.as_ref())?;
        self.block_range.end += 1;
        self.block_macs.push(this_mac);
        Ok(())
    }

    /// Ensures the persistence of data.
    pub fn flush(&self) -> Result<()> {
        self.block_log.flush()
    }

    /// Trim the blocks before a specified position (exclusive).
    ///
    /// The purpose of this method is to free some memory used for keeping the
    /// MACs of accessible blocks. After trimming, the range of accessible
    /// blocks is shrunk accordingly.
    pub fn trim(&mut self, before_block: BlockId) {
        // We must ensure the invariance that there is at least one valid block
        // after trimming.
        debug_assert!(before_block < self.block_range.end);

        if before_block <= self.block_range.start {
            return;
        }

        let num_blocks_trimmed = before_block - self.block_range.start;
        self.block_range.start = before_block;
        self.block_macs.drain(..num_blocks_trimmed);
    }

    /// Returns the range of blocks that are accessible through the `CryptoChain`.
    pub fn block_range(&self) -> &Range<BlockId> {
        &self.block_range
    }

    /// Returns the underlying block log.
    pub fn inner_log(&self) -> &L {
        &self.block_log
    }

    /// Returns the encryption key of the `CryptoChain`.
    pub fn key(&self) -> &Key {
        &self.key
    }
}

/// `Recovery<L>` represents an instance `CryptoChain<L>` being recovered.
///
/// An object `Recovery<L>` attempts to recover as many valid blocks of
/// a `CryptoChain` as possible. A block is valid if and only if its real MAC
/// is equal to the MAC value recorded in its successor.
///
/// For the last block, which does not have a successor block, the user
/// can obtain its MAC from `Recovery<L>` and verify the MAC by comparing it
/// with an expected value from another trusted source.
pub struct Recovery<L> {
    block_log: L,
    key: Key,
    block_range: Range<BlockId>,
    block_macs: Vec<Mac>,
    read_buf: Buf,
    payload: Buf,
}

impl<L: BlockLog> Recovery<L> {
    /// Construct a new `Recovery` from the `first_block` of
    /// `block_log: L`, using a cryptographic `key`.
    pub fn new(block_log: L, key: Key, first_block: BlockId) -> Self {
        Self {
            block_log,
            key,
            block_range: first_block..first_block,
            block_macs: Vec::new(),
            read_buf: Buf::alloc(1).unwrap(),
            payload: Buf::alloc(1).unwrap(),
        }
    }

    /// Returns the number of valid blocks.
    ///
    /// Each success call to `next` increments the number of valid blocks.
    pub fn num_blocks(&self) -> usize {
        self.block_range.len()
    }

    /// Returns the range of valid blocks.
    ///
    /// Each success call to `next` increments the upper bound by one.
    pub fn block_range(&self) -> &Range<BlockId> {
        &self.block_range
    }

    /// Returns the MACs of valid blocks.
    ///
    /// Each success call to `next` pushes the MAC of the new valid block.
    pub fn block_macs(&self) -> &[Mac] {
        &self.block_macs
    }

    /// Open a `CryptoChain<L>` from the recovery object.
    ///
    /// User should call `next` to  retrieve valid blocks as much as possible.
    pub fn open(self) -> CryptoChain<L> {
        CryptoChain {
            block_log: self.block_log,
            key: self.key,
            block_range: self.block_range,
            block_macs: self.block_macs,
        }
    }
}

#[gat]
impl<L: BlockLog> LendingIterator for Recovery<L> {
    type Item<'a> = &'a [u8];

    fn next(&mut self) -> Option<Self::Item<'_>> {
        let next_block_id = self.block_range.end;
        self.block_log
            .read(next_block_id, self.read_buf.as_mut())
            .ok()?;

        // Deserialize footer.
        let footer: Footer =
            Pod::from_bytes(&self.read_buf.as_slice()[CryptoChain::<L>::AVAIL_BLOCK_SIZE..]);
        let payload_len = footer.len as usize;
        if payload_len > CryptoChain::<L>::AVAIL_BLOCK_SIZE {
            return None;
        }

        // Decrypt payload.
        let aead = Aead::new();
        aead.decrypt(
            &self.read_buf.as_slice()[..payload_len],
            &self.key,
            &footer.this_iv,
            &footer.pre_mac,
            &footer.this_mac,
            &mut self.payload.as_mut_slice()[..payload_len],
        )
        .ok()?;

        // Crypto blocks are chained: each block stores not only
        // the MAC of its own, but also the MAC of its previous block.
        // So we need to check whether the two MAC values are the same.
        // There is one exception that the `pre_mac` of the first block
        // is NOT checked.
        if self
            .block_macs()
            .last()
            .is_some_and(|mac| mac.as_bytes() != footer.pre_mac.as_bytes())
        {
            return None;
        }

        self.block_range.end += 1;
        self.block_macs.push(footer.this_mac);
        Some(&self.payload.as_slice()[..payload_len])
    }
}

#[cfg(test)]
mod tests {
    use lending_iterator::LendingIterator;

    use super::CryptoChain;
    use crate::layers::bio::{BlockLog, BlockRing, BlockSet, MemDisk};

    #[test]
    fn new() {
        let disk = MemDisk::create(16).unwrap();
        let block_ring = BlockRing::new(disk);
        block_ring.set_cursor(0);
        let chain = CryptoChain::new(block_ring);

        assert_eq!(chain.block_log.nblocks(), 0);
        assert_eq!(chain.block_range.start, 0);
        assert_eq!(chain.block_range.end, 0);
        assert_eq!(chain.block_macs.len(), 0);
    }

    #[test]
    fn append_trim_and_read() {
        let disk = MemDisk::create(16).unwrap();
        let block_ring = BlockRing::new(disk);
        block_ring.set_cursor(0);
        let mut chain = CryptoChain::new(block_ring);

        let data = [1u8; 1024];
        chain.append(&data[..256]).unwrap();
        chain.append(&data[..512]).unwrap();
        assert_eq!(chain.block_range.end, 2);
        assert_eq!(chain.block_macs.len(), 2);

        chain.trim(1);

        assert_eq!(chain.block_range.start, 1);
        assert_eq!(chain.block_range.end, 2);
        assert_eq!(chain.block_macs.len(), 1);

        let mut buf = [0u8; 1024];
        let len = chain.read(1, &mut buf).unwrap();
        assert_eq!(len, 512);
        assert_eq!(buf[..512], [1u8; 512]);
    }

    #[test]
    fn recover() {
        let disk = MemDisk::create(16).unwrap();
        let key = {
            let sub_disk = disk.subset(0..8).unwrap();
            let block_ring = BlockRing::new(sub_disk);
            block_ring.set_cursor(0);
            let data = [1u8; 1024];
            let mut chain = CryptoChain::new(block_ring);
            for _ in 0..4 {
                chain.append(&data).unwrap();
            }
            chain.flush().unwrap();
            chain.key
        };

        let sub_disk = disk.subset(0..8).unwrap();
        let block_ring = BlockRing::new(sub_disk);
        let mut recover = CryptoChain::recover(key, block_ring, 2);
        while let Some(payload) = recover.next() {
            assert_eq!(payload.len(), 1024);
        }
        let chain = recover.open();
        assert_eq!(chain.block_range(), &(2..4));
        assert_eq!(chain.block_macs.len(), 2);
    }
}
