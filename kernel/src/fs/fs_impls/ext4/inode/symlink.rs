// SPDX-License-Identifier: MPL-2.0

//! Symlink read and write for ext4 inodes.
//!
//! Ext4 distinguishes two storage strategies for symbolic link targets:
//!
//! - **Fast symlink** — targets shorter than 60 bytes are stored inline in the
//!   60-byte `i_block` area of the inode without allocating any data block. One
//!   byte is reserved for a trailing NUL. Unlike a regular
//!   file or directory, a fast symlink must *not* carry the `EXTENTS` flag: its
//!   `i_block` holds raw target bytes, not an extent-tree root, so the extent
//!   reader must never parse it.
//! - **Slow symlink** — longer targets are written to an extent-mapped data
//!   block through the normal page-cache path, exactly like a small file.

use super::{
    super::{feature::FeatureIncompatSet, fs::Ext4, prelude::*, utils},
    FileFlags, Inode, InodeDesc, InodeInner, InodePayload, MAX_FAST_SYMLINK_LEN,
    RAW_BLOCK_PTRS_LEN,
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

    fn new_zeros() -> Self {
        Self {
            block_ptrs: [0; RAW_BLOCK_PTRS_LEN],
        }
    }

    fn write(&mut self, target: &[u8]) {
        debug_assert!(target.len() <= MAX_FAST_SYMLINK_LEN);
        self.block_ptrs.as_mut_bytes()[..target.len()].copy_from_slice(target);
    }

    fn read(&self, len: usize) -> Vec<u8> {
        self.block_ptrs.as_bytes()[..len].to_vec()
    }

    /// Returns the raw `i_block` words so the inline target can be written back
    /// to disk (the descriptor still owns the authoritative `i_block`).
    pub(super) fn block_ptrs(&self) -> [u32; RAW_BLOCK_PTRS_LEN] {
        self.block_ptrs
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

    /// Writes symbolic link target bytes into either fast-inline or slow
    /// (extent-mapped) storage.
    pub(in crate::fs::fs_impls::ext4) fn write_link(&self, target: &str) -> Result<()> {
        if self.type_ != InodeType::SymLink {
            return_errno!(Errno::EINVAL);
        }
        let target_len = target.len();
        if target_len >= BLOCK_SIZE {
            return_errno!(Errno::ENAMETOOLONG);
        }
        let fs = self.fs()?;
        let mut inner = self.inner.write();
        inner.write_link(&fs, target)?;
        inner.set_mtime_ctime(utils::now());
        Ok(())
    }
}

/// Returns whether an on-disk symlink stores its target inline in `i_block`.
///
/// This is Linux's `ext4_inode_is_fast_symlink` rule: the symlink is not
/// extent-mapped and owns no data blocks -- `i_blocks` counts only its
/// extended-attribute block, if any. Judging by block ownership instead of
/// target length keeps a foreign volume's short-but-block-backed slow symlink
/// from being misread as inline bytes.
pub(super) fn is_fast_symlink(desc: &InodeDesc) -> bool {
    desc.type_() == InodeType::SymLink
        && !desc.is_extent_based()
        && desc.sector_count() == xattr_sectors(desc.file_acl())
}

/// Returns how many 512-byte sectors the inode's extended-attribute block
/// contributes to `i_blocks` (0 when there is no EA block).
fn xattr_sectors(file_acl: u64) -> u64 {
    if file_acl == 0 {
        0
    } else {
        (BLOCK_SIZE / SECTOR_SIZE) as u64
    }
}

impl InodeInner {
    /// Returns whether this symlink uses fast (inline) storage.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(super) fn is_fast_symlink(&self) -> bool {
        matches!(self.payload, InodePayload::FastSymlink { .. })
    }

    fn write_link(&mut self, fs: &Arc<Ext4>, target: &str) -> Result<()> {
        let target_len = target.len();

        // Reserve one byte in `i_block` for a trailing NUL.
        if target_len < MAX_FAST_SYMLINK_LEN {
            // Fast path: store the target inline in the `i_block` area. Free any
            // data block held by a previous slow target first.
            if let InodePayload::DataBacked { block_manager, .. } = &self.payload {
                block_manager.truncate_to_byte_len(0)?;
            }
            let mut fast_target = FastSymlinkTarget::new_zeros();
            fast_target.write(target.as_bytes());
            // The `i_block` now holds raw bytes, not an extent root: clear the
            // `EXTENTS` flag so the extent reader never parses the target.
            self.desc.remove_flags(FileFlags::EXTENTS);
            // Mirror the inline bytes into the descriptor's `i_block` so the
            // writeback path persists the target, then publish the payload.
            self.desc.set_raw_block(fast_target.block_ptrs());
            self.payload = InodePayload::FastSymlink {
                target: fast_target,
            };
        } else {
            // Slow path: write through a mapped data block. A symlink
            // created by `create_inode` already arrives `DataBacked` in the
            // volume's format (size 0); only a fast→slow switch needs to
            // rebuild it.
            if !matches!(self.payload, InodePayload::DataBacked { .. }) {
                // The mapping format follows the volume, like `create_inode`:
                // an ext2-format volume must never gain an extent-mapped
                // inode (fsck flags it and Linux cannot read it), so only
                // extent volumes restore the `EXTENTS` flag cleared by the
                // previous fast target. On extent volumes the restore is
                // required: a reload would otherwise misread the extent root
                // as indirect block pointers.
                let extent_based = fs
                    .super_block()
                    .feature_incompat()
                    .contains(FeatureIncompatSet::EXTENTS);
                self.payload = InodePayload::new_data_backed_empty(
                    Arc::downgrade(fs),
                    extent_based,
                    self.file_size(),
                )?;
                if extent_based {
                    self.desc.add_flags(FileFlags::EXTENTS);
                }
            }
            self.prepare_write(0, target_len)?;
            let mut reader = VmReader::from(target.as_bytes()).to_fallible();
            self.page_cache()?.write(0, &mut reader)?;
        }

        self.set_file_size(target_len);
        Ok(())
    }

    fn read_link(&self) -> Result<String> {
        let link_size = self.file_size();

        if let InodePayload::FastSymlink { target } = &self.payload {
            // Exclude the reserved trailing NUL byte from the target.
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

    use super::super::{super::test_utils::Ext4FixtureBuilder, MAX_FAST_SYMLINK_LEN};
    use crate::time::clocks;

    /// A foreign volume's short-but-block-backed slow symlink (no `EXTENTS`
    /// flag, one data block) must decode as slow storage: the fast/slow rule
    /// judges block ownership, not target length. Reading it exercises the
    /// per-inode indirect dispatch on an extents-enabled volume.
    #[ktest]
    fn short_block_backed_symlink_decodes_as_slow() {
        use super::super::super::test_utils::make_indirect_file_inode;

        clocks::init_for_ktest();
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();

        let target = b"short_target";
        let mut block = [0u8; BLOCK_SIZE];
        block[..target.len()].copy_from_slice(target);
        f.write_data_block(100, &block);

        let mut raw = make_indirect_file_inode(100, target.len() as u32);
        raw.mode = 0o120777; // S_IFLNK | 0777
        f.write_raw_inode(20, &raw);

        let link = f.ext4.read_inode(20).unwrap();
        assert!(!link.inner.read().is_fast_symlink());
        assert_eq!(link.read_link().unwrap(), "short_target");
    }

    /// A foreign fast symlink (inline bytes, no data block, no `EXTENTS`
    /// flag) still decodes as inline storage under the block-ownership rule.
    #[ktest]
    fn foreign_fast_symlink_reads_inline() {
        use super::super::{super::test_utils::make_empty_file_inode, RAW_BLOCK_PTRS_LEN};

        clocks::init_for_ktest();
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();

        let mut raw = make_empty_file_inode();
        raw.mode = 0o120777; // S_IFLNK | 0777
        raw.size_lo = 5;
        raw.flags = 0;
        raw.sector_count = 0;
        raw.block = [0u32; RAW_BLOCK_PTRS_LEN];
        for (i, byte) in b"abcde".iter().enumerate() {
            raw.block[i / 4] |= u32::from(*byte) << ((i % 4) * 8);
        }
        f.write_raw_inode(20, &raw);

        let link = f.ext4.read_inode(20).unwrap();
        assert!(link.inner.read().is_fast_symlink());
        assert_eq!(link.read_link().unwrap(), "abcde");
    }

    /// On an ext2-format volume the fast-to-slow symlink switch must build an
    /// indirect mapping and must NOT set the `EXTENTS` flag: an extent-mapped
    /// inode on such a volume is a format violation (fsck flags it, and
    /// e2fsck's "fix" is to stamp the EXTENTS feature onto the superblock,
    /// which then fails the ext2-flavor mount gate).
    #[ktest]
    fn slow_symlink_on_ext2_volume_stays_indirect() {
        use super::super::{
            super::test_utils::make_empty_file_inode, FileFlags, RAW_BLOCK_PTRS_LEN,
        };

        clocks::init_for_ktest();
        let f = Ext4FixtureBuilder::new(2048, 256, 2048)
            .without_extents_feature()
            .with_block_bitmap_metadata_marked()
            .build()
            .unwrap();

        // A foreign-style fast symlink: inline bytes, no data block.
        let mut raw = make_empty_file_inode();
        raw.mode = 0o120777; // S_IFLNK | 0777
        raw.size_lo = 5;
        raw.flags = 0;
        raw.sector_count = 0;
        raw.block = [0u32; RAW_BLOCK_PTRS_LEN];
        for (i, byte) in b"abcde".iter().enumerate() {
            raw.block[i / 4] |= u32::from(*byte) << ((i % 4) * 8);
        }
        f.write_raw_inode(20, &raw);

        let link = f.ext4.read_inode(20).unwrap();
        assert!(link.inner.read().is_fast_symlink());

        // A long target forces the fast-to-slow switch.
        let long_target = "x".repeat(MAX_FAST_SYMLINK_LEN + 10);
        link.write_link(&long_target).unwrap();
        assert_eq!(link.read_link().unwrap(), long_target);
        assert!(!link.inner.read().desc.flags().contains(FileFlags::EXTENTS));

        // The flag stays clear on disk too.
        link.sync_metadata().unwrap();
        let raw = f.read_raw_inode(link.ino());
        assert_eq!(raw.flags & 0x0008_0000, 0); // EXTENTS_FL
    }

    /// A fast symlink that owns an extended-attribute block: `i_blocks`
    /// counts only the EA block, so the block-ownership rule must still
    /// classify it as inline (Linux's `ext4_inode_is_fast_symlink` rule).
    #[ktest]
    fn fast_symlink_with_xattr_block_stays_fast() {
        use super::super::{super::test_utils::make_empty_file_inode, RAW_BLOCK_PTRS_LEN};

        clocks::init_for_ktest();
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();

        let mut raw = make_empty_file_inode();
        raw.mode = 0o120777; // S_IFLNK | 0777
        raw.size_lo = 5;
        raw.flags = 0;
        raw.file_acl_lo = 500;
        // The EA block is the only block the inode owns.
        raw.sector_count = (BLOCK_SIZE / SECTOR_SIZE) as u32;
        raw.block = [0u32; RAW_BLOCK_PTRS_LEN];
        for (i, byte) in b"abcde".iter().enumerate() {
            raw.block[i / 4] |= u32::from(*byte) << ((i % 4) * 8);
        }
        f.write_raw_inode(20, &raw);

        let link = f.ext4.read_inode(20).unwrap();
        assert!(link.inner.read().is_fast_symlink());
        assert_eq!(link.read_link().unwrap(), "abcde");
    }
}
