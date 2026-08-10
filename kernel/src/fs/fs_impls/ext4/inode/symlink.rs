// SPDX-License-Identifier: MPL-2.0

//! Symlink read for ext4 inodes.
//!
//! Ext4 distinguishes two storage strategies for symbolic link targets:
//!
//! - **Fast symlink** — targets shorter than 60 bytes are stored inline in the
//!   60-byte `i_block` area of the inode without allocating any data block. One
//!   byte is reserved for the Linux-compatible trailing NUL. Unlike a regular
//!   file or directory, a fast symlink must *not* carry the `EXTENTS` flag: its
//!   `i_block` holds raw target bytes, not an extent-tree root, so the extent
//!   reader must never parse it.
//! - **Slow symlink** — longer targets are written to an extent-mapped data
//!   block through the normal page-cache path, exactly like a small file.

use super::{
    super::prelude::*, Inode, InodeInner, InodePayload, MAX_FAST_SYMLINK_LEN, RAW_BLOCK_PTRS_LEN,
};

/// Inline fast-symlink target stored in the raw `i_block` byte area.
#[derive(Debug)]
pub(super) struct FastSymlinkTarget {
    block_ptrs: [u32; RAW_BLOCK_PTRS_LEN],
}

impl FastSymlinkTarget {
    pub(super) fn new(block_ptrs: [u32; RAW_BLOCK_PTRS_LEN]) -> Self {
        Self { block_ptrs }
    }

    fn read(&self, len: usize) -> Vec<u8> {
        self.block_ptrs.as_bytes()[..len].to_vec()
    }
}

impl Inode {
    /// Reads symbolic link target bytes and decodes them as UTF-8.
    pub(in crate::fs::fs_impls::ext4) fn read_link(&self) -> Result<String> {
        if self.type_ != InodeType::SymLink {
            return_errno!(Errno::EINVAL);
        }
        self.inner.read().read_link()
    }
}

impl InodeInner {
    /// Returns whether this symlink uses fast (inline) storage.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(super) fn is_fast_symlink(&self) -> bool {
        matches!(self.payload, InodePayload::FastSymlink { .. })
    }

    fn read_link(&self) -> Result<String> {
        let link_size = self.file_size();

        if let InodePayload::FastSymlink { target } = &self.payload {
            // Exclude the Linux-compatible trailing NUL byte from the target.
            let read_len = link_size.min(MAX_FAST_SYMLINK_LEN - 1);
            let target_bytes = target.read(read_len);
            return String::from_utf8(target_bytes)
                .map_err(|_| Error::with_message(Errno::EIO, "symlink target is not valid UTF-8"));
        }

        let mut buf = vec![0u8; link_size];
        let mut writer = VmWriter::from(buf.as_mut_slice()).to_fallible();
        self.page_cache()?.read(0, &mut writer).map_err(|_| {
            Error::with_message(Errno::EIO, "failed to read symlink target from page cache")
        })?;

        String::from_utf8(buf)
            .map_err(|_| Error::with_message(Errno::EIO, "symlink target is not valid UTF-8"))
    }
}

#[cfg(ktest)]
mod tests {
    use aster_block::{BLOCK_SIZE, SECTOR_SIZE};
    use ostd::prelude::*;

    use super::super::{
        super::test_utils::{Ext4Fixture, Ext4FixtureBuilder},
        FileFlags, MAX_FAST_SYMLINK_LEN, RawInode,
    };
    use crate::{fs::file::InodeType, prelude::Errno};

    const LINK_INO: u32 = 11;

    /// Lays down a non-extent symlink at `ino`: its `i_block` holds raw target
    /// bytes rather than an extent-tree root.
    fn write_non_extent_symlink(f: &Ext4Fixture, ino: u32, target: &str) {
        debug_assert!(target.len() <= MAX_FAST_SYMLINK_LEN);
        let mut raw = RawInode {
            mode: 0o120777, // S_IFLNK | 0777
            size_lo: target.len() as u32,
            link_count: 1,
            sector_count: 0,
            flags: 0, // a fast symlink carries no EXTENTS flag; i_block is raw bytes
            extra_isize: 32,
            ..Default::default()
        };
        // Pack the target into the 60-byte `i_block` area, little-endian words —
        // the same on-disk layout `FastSymlinkTarget::read` decodes.
        for (i, chunk) in target.as_bytes().chunks(4).enumerate() {
            let mut word = [0u8; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            raw.block[i] = u32::from_le_bytes(word);
        }
        f.write_raw_inode(ino, &raw);
    }

    /// Lays down a *fast* (inline) symlink at `ino`. This helper accepts only
    /// targets that fit strictly within the 60-byte `i_block` area.
    fn write_fast_symlink(f: &Ext4Fixture, ino: u32, target: &str) {
        debug_assert!(target.len() < MAX_FAST_SYMLINK_LEN);
        write_non_extent_symlink(f, ino, target);
    }

    /// Lays down a *slow* (block-backed) symlink at `ino`: an extent-mapped
    /// symlink inode whose single data block holds the target, exactly how a
    /// long target is stored on disk.
    fn write_slow_symlink(f: &Ext4Fixture, ino: u32, data_block: u32, target: &str) {
        let mut raw = RawInode {
            mode: 0o120777, // S_IFLNK | 0777
            size_lo: target.len() as u32,
            link_count: 1,
            sector_count: (BLOCK_SIZE / SECTOR_SIZE) as u32,
            flags: FileFlags::EXTENTS.bits(),
            extra_isize: 32,
            ..Default::default()
        };
        // Inline extent root mapping logical block 0 to `data_block`, length 1.
        raw.block[0] = 0xF30A | (1 << 16); // eh_magic | eh_entries
        raw.block[1] = 4; // eh_max=4, eh_depth=0
        raw.block[3] = 0; // ee_block = 0
        raw.block[4] = 1; // ee_len=1, ee_start_hi=0
        raw.block[5] = data_block; // ee_start_lo
        f.write_data_block(data_block, target.as_bytes());
        f.write_raw_inode(ino, &raw);
    }

    /// A short target (< 60 bytes) stored inline in `i_block` is decoded as a
    /// fast symlink and `read_link` round-trips it. The inode is not
    /// extent-based, so the extent reader never parses the raw target bytes.
    #[ktest]
    fn fast_symlink_reads_inline_target() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();
        let target = "x".repeat(MAX_FAST_SYMLINK_LEN - 1);
        write_fast_symlink(&f, LINK_INO, &target);

        let link = f.ext4.read_inode(LINK_INO).unwrap();
        assert_eq!(link.inode_type(), InodeType::SymLink);
        assert_eq!(link.size(), target.len());
        {
            let inner = link.inner.read();
            assert!(inner.is_fast_symlink());
            // No EXTENTS flag: the target is raw bytes, never an extent root.
            assert!(!inner.desc.is_extent_based());
        }
        // A fast symlink consumes no sectors.
        assert_eq!(link.sector_count(), 0);
        assert_eq!(link.read_link().unwrap(), target);
    }

    /// A non-extent symlink with size exactly 60 is not a fast symlink: ext4
    /// reserves one byte for the trailing NUL. It therefore falls through to the
    /// data-backed path, where the raw target bytes fail extent-root parsing.
    #[ktest]
    fn symlink_size_60_is_rejected_as_malformed_slow_symlink() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();
        let target = "x".repeat(MAX_FAST_SYMLINK_LEN);
        write_non_extent_symlink(&f, LINK_INO, &target);

        let Err(err) = f.ext4.read_inode(LINK_INO) else {
            panic!("non-extent size-60 symlink must not decode as fast");
        };
        assert_eq!(err.error(), Errno::EUCLEAN);
    }

    /// A long target (> 60 bytes, < BLOCK_SIZE) stored in an extent-mapped data
    /// block is decoded as a slow symlink and `read_link` round-trips it through
    /// the page-cache read path.
    #[ktest]
    fn slow_symlink_reads_data_block_target() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();
        let target = "x".repeat(200); // > MAX_FAST_SYMLINK_LEN, < BLOCK_SIZE
        assert!(target.len() >= MAX_FAST_SYMLINK_LEN && target.len() < BLOCK_SIZE);
        write_slow_symlink(&f, LINK_INO, 100, &target);

        let link = f.ext4.read_inode(LINK_INO).unwrap();
        assert_eq!(link.inode_type(), InodeType::SymLink);
        assert_eq!(link.size(), target.len());
        {
            let inner = link.inner.read();
            assert!(!inner.is_fast_symlink());
            // A block-backed symlink keeps the EXTENTS flag.
            assert!(inner.desc.is_extent_based());
        }
        assert_eq!(link.sector_count(), (BLOCK_SIZE / SECTOR_SIZE) as u64);
        assert_eq!(link.read_link().unwrap(), target);
    }

    /// `read_link` rejects a non-symlink inode (the root directory) with an error.
    #[ktest]
    fn rejects_read_link_on_non_symlink() {
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();
        // The root (ino 2) is a directory, not a symlink.
        let dir = f.ext4.read_inode(2).unwrap();
        assert!(dir.read_link().is_err());
    }
}
