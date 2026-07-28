// SPDX-License-Identifier: MPL-2.0

//! Writeback and reclaim for ext4 inodes.
//!
//! This module contains the inode paths that make in-memory state durable and
//! reclaim deleted inodes. Writeback must keep xattrs, data pages, engine
//! metadata, and inode-slot state ordered consistently; reclaim must release
//! all storage owned by a zero-link inode exactly once.
//!
//! The `*_no_barrier` combinators exist so the filesystem-level sync can flush
//! every cached inode and the block-side metadata before issuing a single
//! device barrier, rather than one barrier per inode.

use super::{
    super::{fs::Ext4, prelude::*, utils},
    Inode, InodeInner, InodePayload,
};

impl Inode {
    /// Persists the inode's mutable metadata (size, `i_blocks`, extent root,
    /// timestamps) to disk if dirty. Data pages are flushed by
    /// [`sync_data_and_meta`](Self::sync_data_and_meta).
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(in crate::fs::fs_impls::ext4) fn sync_metadata(&self) -> Result<()> {
        let fs = self.fs()?;
        let mut inner = self.inner.write();
        inner.write_back_inode_desc(&fs, self.ino)
    }

    /// Flushes dirty data pages, then the inode metadata, then issues a device
    /// sync — leaving a consistent on-disk image (data → inode → barrier; mirrors
    /// ext2 `sync.rs` ordering).
    pub(in crate::fs::fs_impls::ext4) fn sync_data_and_meta(&self) -> Result<()> {
        self.sync_data_and_meta_no_barrier()?;
        let fs = self.fs()?;
        if fs.block_device().sync()? != BioStatus::Complete {
            return_errno_with_message!(Errno::EIO, "failed to flush block device");
        }
        Ok(())
    }

    /// Flushes dirty data pages and then the inode metadata, *without* a device
    /// barrier. Used by the filesystem-level sync, which flushes every cached
    /// inode and the block-side metadata before issuing a single barrier — so a
    /// per-inode barrier here would be redundant.
    pub(super) fn sync_data_and_meta_no_barrier(&self) -> Result<()> {
        let fs = self.fs()?;
        let mut inner = self.inner.write();
        inner.sync_data_pages()?;
        inner.write_back_inode_desc(&fs, self.ino)?;
        Ok(())
    }

    /// Flushes the xattr block, if the inode owns a dirty one.
    ///
    /// Ordered before the inode-table writeback so the `i_file_acl` pointer
    /// never becomes durable before the block it names. Never called with
    /// `inner` held (`xattr` and `inner` locks are never nested).
    pub(in crate::fs::fs_impls::ext4) fn flush_xattr(&self) -> Result<()> {
        match &self.xattr {
            Some(xattr) => xattr.flush(),
            None => Ok(()),
        }
    }

    /// Flushes xattr, data pages, and inode metadata without a device barrier.
    /// Used by the filesystem-level sync, which issues a single barrier after
    /// flushing every cached inode.
    pub(in crate::fs::fs_impls::ext4) fn sync_all_no_barrier(&self) -> Result<()> {
        self.flush_xattr()?;
        self.sync_data_and_meta_no_barrier()
    }

    /// Reclaims a fully unlinked inode: frees its data blocks and inode bit.
    ///
    /// Runs from `Drop` when the last `Arc<Inode>` is released. A no-op (returns
    /// `Ok(false)`) unless the inode's link count is 0 *and* its bitmap bit is
    /// still allocated — the latter guards against double-freeing an inode an
    /// earlier reclaim already released. On reclaim it frees the
    /// extended-attribute block, stamps `i_dtime`, drops the data (page cache +
    /// extent-mapped blocks, only for data-backed inodes — a fast symlink has
    /// no data block), persists the descriptor, and frees the inode.
    pub(super) fn try_reclaim_deleted_inode(&self) -> Result<bool> {
        if self.link_count() != 0 {
            return Ok(false);
        }

        let fs = self.fs()?;
        if !fs.is_inode_allocated(self.ino) {
            return Ok(false);
        }

        if let Some(xattr) = self.xattr.as_ref() {
            xattr.delete_xattr_block()?;
        }

        let mut inner = self.inner.write();
        let old_size = inner.file_size();
        // Only data-backed inodes (files, directories, slow symlinks) own a page
        // cache and extent-mapped blocks. A fast symlink stores its target inline
        // in `i_block` with no data block, so it skips both the page-cache resize
        // and the block truncate below.
        let block_manager = inner.block_manager().ok().cloned();
        if block_manager.is_some() {
            inner.resize_page_cache(0, old_size)?;
        }
        inner.set_dtime(utils::now());
        inner.set_file_size(0);
        inner.set_file_acl(0);
        // Gate on the extent manager's live `sector_count`, not the descriptor's
        // copy (which ext2 uses): the extent manager is the authority and the
        // descriptor may be stale until writeback. This divergence from the ext2
        // template is intentional — do not "fix" it back to `inner.desc`.
        if let Some(block_manager) = block_manager
            && block_manager.sector_count() > 0
        {
            block_manager.truncate_to_byte_len(0)?;
        }
        inner.write_back_inode_desc(&fs, self.ino)?;

        fs.free_inode(self.ino, self.type_)?;
        Ok(true)
    }
}

impl InodeInner {
    /// Persists the inode's mutable metadata to its on-disk `RawInode` if dirty,
    /// pulling the extent root and `i_blocks` from the block manager, and clears
    /// the dirty flags.
    pub(super) fn write_back_inode_desc(&mut self, fs: &Ext4, ino: Ext4Ino) -> Result<()> {
        if !self.is_dirty() {
            return Ok(());
        }
        // Engine-internal metadata blocks must be durable before the inode
        // that references them is written back.
        if let Ok(bm) = self.block_manager() {
            bm.sync_meta()?;
        }
        let (root, sector_count) = match self.block_manager() {
            Ok(bm) => (bm.root_snapshot(), bm.sector_count()),
            Err(_) => (*self.desc.raw_block(), self.desc.sector_count()),
        };
        // Mirror the authoritative `i_blocks` into the descriptor before writing.
        self.desc.set_sector_count(sector_count);
        fs.write_back_inode_desc(ino, &self.desc, &root)?;
        self.clear_dirty();
        Ok(())
    }

    /// Flushes dirty data pages in `[0, file_size)`.
    fn sync_data_pages(&self) -> Result<()> {
        let file_size = self.file_size();
        if file_size == 0 {
            return Ok(());
        }
        match &self.payload {
            InodePayload::DataBacked { page_cache, .. } => page_cache.flush_range(0..file_size),
            _ => Ok(()),
        }
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::ktest;

    use super::super::{
        super::test_utils::{Ext4Fixture, Ext4FixtureBuilder, make_empty_file_inode},
        Inode,
    };
    use crate::{prelude::*, time::clocks};

    const FILE_INO: u32 = 11;

    /// A fixture with a realistic bitmap and an empty regular file at `FILE_INO`.
    fn fixture_with_empty_file() -> Ext4Fixture {
        clocks::init_for_ktest();
        let f = Ext4FixtureBuilder::new(2048, 256, 2048)
            .with_block_bitmap_metadata_marked()
            .build()
            .unwrap();
        f.write_raw_inode(FILE_INO, &make_empty_file_inode());
        f
    }

    fn write_all(inode: &Inode, offset: usize, data: &[u8]) -> usize {
        let mut reader = VmReader::from(data).to_fallible();
        inode.write_at(offset, &mut reader).unwrap()
    }

    /// The inode-slot read-modify-write preserves on-disk fields the driver does
    /// not model (generation, checksum, extra-size region, creation time) while
    /// updating the ones a write touches.
    #[ktest]
    fn write_back_inode_desc_is_lossless() {
        let f = fixture_with_empty_file();

        // Seed a few distinctive immutable fields the RMW must preserve.
        let mut raw = make_empty_file_inode();
        raw.generation = 0xDEAD_BEEF;
        raw.checksum_lo = 0x1234;
        raw.extra_isize = 32;
        raw.uid = 0x1111;
        raw.uid_high = 0x2222;
        f.write_raw_inode(FILE_INO, &raw);
        let before = f.read_raw_inode(FILE_INO);

        let inode = f.ext4.read_inode(FILE_INO).unwrap();
        write_all(&inode, 0, b"persisted");
        inode.sync_metadata().unwrap();

        let after = f.read_raw_inode(FILE_INO);

        // Mutated fields changed.
        assert_eq!(after.size_lo, b"persisted".len() as u32);
        assert!(after.sector_count > 0);
        assert_ne!(after.block, before.block); // extent root rewritten

        // Untouched fields preserved.
        assert_eq!(after.generation, before.generation);
        assert_eq!(after.checksum_lo, before.checksum_lo);
        assert_eq!(after.extra_isize, before.extra_isize);
        assert_eq!(after.uid, before.uid);
        assert_eq!(after.uid_high, before.uid_high);
        assert_eq!(after.crtime, before.crtime);
        assert_eq!(after.crtime_extra, before.crtime_extra);
    }
}
