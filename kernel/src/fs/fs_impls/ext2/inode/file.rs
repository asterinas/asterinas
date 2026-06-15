// SPDX-License-Identifier: MPL-2.0

//! Regular-file data I/O for ext2 inodes: buffered reads/writes and fallocate.
//!
//! This module contains the regular-file behavior exposed through the VFS
//! layer: page-cache I/O, direct I/O, size changes, and block preallocation.
//! File size, page-cache state, and ext2 block mappings must stay coherent
//! across those entry points.

use ostd::mm::io::util::HasVmReaderWriter;

use super::{super::Ext2, FileFlags, Inode, InodeInner, io_range::IoRange};
use crate::fs::{
    ext2::{prelude::*, utils},
    vfs::inode::FallocMode,
};

impl Inode {
    /// Reads file data at `offset` through the page cache.
    pub(in crate::fs::fs_impls::ext2) fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
    ) -> Result<usize> {
        if self.type_ == InodeType::Dir {
            return_errno!(Errno::EISDIR);
        }

        if writer.avail() == 0 {
            return Ok(0);
        }

        let read_len = self.inner.read().read_at(offset, writer)?;
        if read_len > 0 {
            self.inner.write().set_atime(utils::now());
        }
        Ok(read_len)
    }

    /// Writes file data at `offset` through the page cache.
    pub(in crate::fs::fs_impls::ext2) fn write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
    ) -> Result<usize> {
        if self.type_ == InodeType::Dir {
            return_errno!(Errno::EISDIR);
        }

        let write_len = reader.remain();
        if write_len == 0 {
            return Ok(0);
        }

        let fs = self.fs()?;
        let mut inner = self.inner.write();
        inner.write_at(&fs, offset, reader)
    }

    /// Direct-I/O read path.
    pub(in crate::fs::fs_impls::ext2) fn read_direct_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
    ) -> Result<usize> {
        if self.type_ == InodeType::Dir {
            return_errno!(Errno::EISDIR);
        }

        let read_len = writer.avail();
        if !is_block_aligned(offset) || !is_block_aligned(read_len) {
            // TODO: Implement a fallback mechanism.
            return_errno_with_message!(Errno::EINVAL, "not block-aligned");
        }
        if read_len == 0 {
            return Ok(0);
        }

        let fs = self.fs()?;
        let read_len = self.inner.read().read_direct_at(&fs, offset, writer)?;
        if read_len > 0 {
            self.inner.write().set_atime(utils::now());
        }
        Ok(read_len)
    }

    /// Direct-I/O write path with pre-allocation and rollback.
    pub(in crate::fs::fs_impls::ext2) fn write_direct_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
    ) -> Result<usize> {
        if self.type_ == InodeType::Dir {
            return_errno!(Errno::EISDIR);
        }

        let write_len = reader.remain();
        if write_len == 0 {
            return Ok(0);
        }

        if !is_block_aligned(offset) || !is_block_aligned(write_len) {
            // TODO: Implement a fallback mechanism.
            return_errno_with_message!(Errno::EINVAL, "not block-aligned");
        }

        let fs = self.fs()?;
        let mut inner = self.inner.write();
        inner.write_direct_at(&fs, offset, reader)
    }

    /// Truncates or extends the file to `new_size` bytes.
    pub(in crate::fs::fs_impls::ext2) fn resize(&self, new_size: usize) -> Result<()> {
        if self.type_ == InodeType::Dir {
            return_errno!(Errno::EISDIR);
        }
        if self.type_ != InodeType::File && self.type_ != InodeType::SymLink {
            return_errno!(Errno::EINVAL);
        }

        let fs = self.fs()?;
        let mut inner = self.inner.write();
        if inner.is_fast_symlink() && inner.file_size() != 0 {
            return_errno!(Errno::EINVAL);
        }
        if inner
            .desc
            .flags
            .intersects(FileFlags::APPEND_ONLY | FileFlags::IMMUTABLE)
        {
            return_errno!(Errno::EPERM);
        }

        let old_size = inner.file_size();
        if new_size == old_size {
            return Ok(());
        }

        if new_size < old_size {
            inner.shrink(new_size)?;
        } else {
            inner.expand(&fs, new_size)?;
        }
        inner.set_mtime_ctime(utils::now());
        Ok(())
    }

    /// Implements fallocate operations for ext2.
    pub(in crate::fs::fs_impls::ext2) fn fallocate(
        &self,
        mode: FallocMode,
        offset: usize,
        len: usize,
    ) -> Result<()> {
        if len == 0 {
            return Ok(());
        }
        let end = offset
            .checked_add(len)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "fallocate range overflow"))?;
        let fs = self.fs()?;
        let mut inner = self.inner.write();
        let old_size = inner.file_size();
        if end > old_size {
            inner.ensure_size_within_limit(&fs, end)?;
        }

        match mode {
            FallocMode::Allocate => {
                if let Err(err) = inner.allocate_range_blocks(offset, end) {
                    inner.rollback_fallocate(old_size);
                    return Err(err);
                }
                if end > old_size
                    && let Err(err) = inner.expand(&fs, end)
                {
                    inner.rollback_fallocate(old_size);
                    return Err(err);
                }
            }
            FallocMode::AllocateKeepSize => {
                if let Err(err) = inner.allocate_range_blocks(offset, end) {
                    inner.rollback_fallocate(old_size);
                    return Err(err);
                }
            }
            _ => {
                return_errno_with_message!(
                    Errno::EOPNOTSUPP,
                    "fallocate with the specified flags is not supported"
                );
            }
        }

        inner.set_mtime_ctime(utils::now());
        Ok(())
    }
}

impl InodeInner {
    /// Prepares the inode for a write spanning `[offset, end)`.
    ///
    /// Expands the page cache if `end > file_size`, zeroes partial block
    /// boundaries, and allocates missing data blocks.
    ///
    /// # Rollback
    ///
    /// On failure the caller **must** invoke `rollback_write` with the
    /// original file size and `end` to restore page-cache capacity and
    /// free partially allocated blocks. The caller must hold `InodeInner`
    /// write lock for the entire prepare-write-commit sequence.
    pub(super) fn prepare_write(&mut self, fs: &Ext2, offset: usize, end: usize) -> Result<()> {
        let old_size = self.file_size();
        if end > old_size {
            self.ensure_size_within_limit(fs, end)?;
            self.resize_page_cache(end, old_size)?;
        }

        self.allocate_range_blocks(offset, end)?;
        Ok(())
    }

    /// Truncates page cache and blocks after a failed write.
    pub(super) fn rollback_write(&mut self, old_size: usize, end: usize) {
        if end <= old_size {
            return;
        }
        if let Err(err) = self.resize_page_cache(old_size, end) {
            error!(
                "write_at: cleanup page cache resize failed: old_size={}, err={:?}",
                old_size, err
            );
        }

        if let Ok(block_manager) = self.block_manager() {
            block_manager.truncate_to_byte_len(old_size);
        }
    }

    /// Truncates only block mappings after a failed fallocate.
    fn rollback_fallocate(&mut self, old_size: usize) {
        if let Ok(block_manager) = self.block_manager() {
            block_manager.truncate_to_byte_len(old_size);
        }
    }

    /// Reads file data at `offset` through the inode page cache.
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        if writer.avail() == 0 {
            return Ok(0);
        }

        let file_size = self.file_size();
        if offset >= file_size {
            return Ok(0);
        }

        let read_len = writer.avail().min(file_size - offset);
        writer.limit(read_len);
        self.page_cache().read(offset, writer)?;
        Ok(read_len)
    }

    /// Writes file data at `offset` through the inode page cache.
    fn write_at(&mut self, fs: &Ext2, offset: usize, reader: &mut VmReader) -> Result<usize> {
        let write_len = reader.remain();
        if write_len == 0 {
            return Ok(0);
        }

        let end = offset
            .checked_add(write_len)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "write range overflow"))?;
        let old_size = self.file_size();

        if let Err(err) = self.prepare_write(fs, offset, end) {
            self.rollback_write(old_size, end);
            return Err(err);
        }

        if let Err(err) = self.page_cache().write(offset, reader) {
            self.rollback_write(old_size, end);
            return Err(err.into());
        }

        self.set_mtime_ctime(utils::now());
        if end > old_size {
            self.set_file_size(end);
        }
        Ok(write_len)
    }

    /// Reads file data directly after flushing overlapping cached pages.
    fn read_direct_at(&self, fs: &Ext2, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let file_size = self.file_size();
        if offset >= file_size || writer.avail() == 0 {
            return Ok(0);
        }

        let read_len = writer.avail().min(file_size - offset);
        writer.limit(read_len);
        let end = offset + read_len;

        self.page_cache().flush_range(offset..end)?;
        self.read_direct_blocks(fs, offset, end, writer)?;
        Ok(read_len)
    }

    /// Writes file data directly after invalidating overlapping cached pages.
    fn write_direct_at(
        &mut self,
        fs: &Ext2,
        offset: usize,
        reader: &mut VmReader,
    ) -> Result<usize> {
        let write_len = reader.remain();
        if write_len == 0 {
            return Ok(0);
        }

        let end = offset
            .checked_add(write_len)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "write range overflow"))?;
        let old_size = self.file_size();

        if let Err(err) = self.prepare_write(fs, offset, end) {
            self.rollback_write(old_size, end);
            return Err(err);
        }

        let discard_start_bytes = offset.min(old_size);
        let discard_end_bytes = end.min(old_size);
        if discard_start_bytes < discard_end_bytes {
            self.page_cache()
                .invalidate_range(discard_start_bytes..discard_end_bytes)?;
        }

        if let Err(err) = self.write_direct_blocks(fs, offset, reader) {
            self.rollback_write(old_size, end);
            return Err(err);
        }

        self.set_mtime_ctime(utils::now());
        if end > self.file_size() {
            self.set_file_size(end);
        }
        Ok(write_len)
    }

    /// Reads file data directly from data blocks into `writer`.
    fn read_direct_blocks(
        &self,
        fs: &Ext2,
        offset: usize,
        end: usize,
        writer: &mut VmWriter,
    ) -> Result<()> {
        let iblock_start = Iblock::try_from(offset / BLOCK_SIZE)
            .map_err(|_| Error::with_message(Errno::EINVAL, "logical block number overflow"))?;
        let iblock_end = Iblock::try_from(end.div_ceil(BLOCK_SIZE))
            .map_err(|_| Error::with_message(Errno::EINVAL, "logical block number overflow"))?;
        let iblock_range = iblock_start..iblock_end;
        let mut range_iter = self.block_manager()?.iter_io_ranges(iblock_range);

        while let Some(range) = range_iter.next()? {
            match range {
                IoRange::Mapped(device_range) => {
                    let nblocks = device_range.len();
                    let bio_segment = BioSegment::alloc(nblocks, BioDirection::FromDevice);
                    fs.read_blocks(device_range.start, bio_segment.clone())?;
                    let mut segment_reader = bio_segment.reader()?;
                    segment_reader.read_fallible(writer)?;
                }
                IoRange::Hole(hole_range) => {
                    let n_bytes =
                        (hole_range.end as usize - hole_range.start as usize) * BLOCK_SIZE;
                    writer.fill_zeros(n_bytes)?;
                }
            }
        }
        Ok(())
    }

    /// Writes file data directly to already-allocated data blocks.
    fn write_direct_blocks(
        &mut self,
        fs: &Ext2,
        offset: usize,
        reader: &mut VmReader,
    ) -> Result<()> {
        let write_len = reader.remain();
        debug_assert_eq!(write_len % BLOCK_SIZE, 0);
        // `end` is already checked in `InodeInner::write_direct_at`.
        let end = offset + write_len;
        let iblock_start = Iblock::try_from(offset / BLOCK_SIZE)
            .map_err(|_| Error::with_message(Errno::EINVAL, "logical block number overflow"))?;
        let iblock_end = Iblock::try_from(end.div_ceil(BLOCK_SIZE))
            .map_err(|_| Error::with_message(Errno::EINVAL, "logical block number overflow"))?;
        let iblock_range = iblock_start..iblock_end;
        let mut range_iter = self.block_manager()?.iter_io_ranges(iblock_range);

        let mut io_batch = IoBatch::new();
        while let Some(range) = range_iter.next()? {
            match range {
                IoRange::Mapped(device_range) => {
                    let nblocks = device_range.len();
                    let bio_segment = BioSegment::alloc(nblocks, BioDirection::ToDevice);
                    bio_segment.writer().unwrap().write_fallible(reader)?;
                    fs.write_blocks_async(device_range.start, bio_segment, None, &mut io_batch)?;
                }
                IoRange::Hole(_) => {
                    // TODO: Consider falling back to buffered write like Linux.
                    // The upper layer should have performed allocation for the write
                    // range. Linux does not allocate blocks in the direct write path;
                    // when it encounters a hole it falls back to buffered write to
                    // prevent stale data exposure. We pre-allocate in `prepare_write`
                    // so holes here indicate a bug. Stale-read is not a concern
                    // because our inode-level lock serializes reads after this write.
                    return_errno_with_message!(Errno::EIO, "unexpected hole in direct write path");
                }
            }
        }

        io_batch.wait_all()?;
        Ok(())
    }

    fn shrink(&mut self, new_size: usize) -> Result<()> {
        let old_size = self.desc.size as usize;

        self.resize_page_cache(new_size, old_size)?;
        self.block_manager()?.truncate_to_byte_len(new_size);

        self.set_file_size(new_size);
        Ok(())
    }

    fn expand(&mut self, fs: &Ext2, new_size: usize) -> Result<()> {
        let old_size = self.file_size();

        if new_size <= old_size {
            return Ok(());
        }
        self.ensure_size_within_limit(fs, new_size)?;
        self.resize_page_cache(new_size, old_size)?;

        self.set_file_size(new_size);
        Ok(())
    }

    /// Allocates any missing data blocks covering the requested file byte range.
    fn allocate_range_blocks(&mut self, offset: usize, end: usize) -> Result<()> {
        let start_block = offset / BLOCK_SIZE;
        let end_block = end.div_ceil(BLOCK_SIZE);
        self.block_manager()?
            .allocate_range_blocks(start_block, end_block)
    }

    /// Rejects growth beyond the ext2-representable size limit before mutating state.
    fn ensure_size_within_limit(&self, fs: &Ext2, new_size: usize) -> Result<()> {
        let max_size = match self.inode_type() {
            InodeType::File => fs.max_file_size(),
            _ => u32::MAX as usize,
        };
        if new_size > max_size {
            return_errno_with_message!(Errno::EFBIG, "inode size exceeds ext2 maximum");
        }

        Ok(())
    }
}

fn is_block_aligned(offset: usize) -> bool {
    offset.is_multiple_of(BLOCK_SIZE)
}

#[cfg(ktest)]
mod test {
    use ostd::prelude::ktest;

    use super::{super::RAW_BLOCK_PTRS_LEN, *};
    use crate::{
        fs::ext2::{
            inode::test::make_live_file_inode,
            test_utils::{Ext2FixtureBuilder, assert_errno, create_file},
        },
        time::clocks,
    };

    #[ktest]
    fn file_write_enospc_rollback() {
        clocks::init_for_ktest();

        let f = Ext2FixtureBuilder::new(1, 256)
            .with_free_blocks(2, 2)
            .with_free_inodes(1000, 1000)
            .with_group0_used_dirs(1)
            .build()
            .unwrap();
        let root = f.root();
        let file = create_file(&root, "enospc");
        let base_data = vec![0x44u8; BLOCK_SIZE];

        let mut reader = VmReader::from(base_data.as_slice()).to_fallible();
        file.write_direct_at(0, &mut reader).unwrap();
        let free_before_fail = f.ext2.super_block().free_blocks_count();
        assert_eq!(free_before_fail, 1);

        let fail_payload = vec![0x66u8; BLOCK_SIZE * 2];
        let mut fail_reader = VmReader::from(fail_payload.as_slice()).to_fallible();
        assert_errno!(
            file.write_direct_at(BLOCK_SIZE, &mut fail_reader),
            Errno::ENOSPC
        );

        assert_eq!(file.file_size(), BLOCK_SIZE);
        assert_eq!(f.ext2.super_block().free_blocks_count(), free_before_fail);

        let mut readback = vec![0u8; BLOCK_SIZE];
        let mut writer = VmWriter::from(readback.as_mut_slice()).to_fallible();
        file.read_direct_at(0, &mut writer).unwrap();
        assert_eq!(readback, base_data);
    }

    // TODO: Enable this test once page-table dirty bits are propagated back to
    // the VMO. Currently the hardware dirty flag set by mmap writes is not
    // reflected in the VMO's dirty tracking, so a subsequent buffered write to
    // the same page overwrites the mmap-dirtied region with zeros (the page
    // cache sees the page as clean and re-zeroes the hole portion). Once the
    // VM subsystem flushes PTE dirty bits back to the VMO, this test should
    // pass and can be re-enabled.
    // #[ktest]
    // fn file_sparse_buffered_write_preserves_mmap_dirty_tail() {
    //     let (_f, root) = default_fixture();
    //     let file = create_file(&root, "mmap_dirty_tail");
    //     let vmo = VfsInodeTrait::page_cache(file.as_ref()).unwrap().as_vmo();

    //     VfsInodeTrait::resize(file.as_ref(), BLOCK_SIZE * 3).unwrap();

    //     let block_start = BLOCK_SIZE;
    //     let mmap_offset = block_start + 200;
    //     let mmap_payload = [0x5au8; 32];

    //     let page = vmo.commit_on(block_start / PAGE_SIZE).unwrap();
    //     page.write_bytes(mmap_offset % PAGE_SIZE, &mmap_payload)
    //         .unwrap();
    //     vmo.mark_page_dirty(block_start).unwrap();

    //     let buffered_offset = block_start + 100;
    //     let buffered_payload = [0xa5u8; 100];
    //     assert_eq!(
    //         write_file_at(
    //             &file,
    //             buffered_offset,
    //             &buffered_payload,
    //             StatusFlags::empty()
    //         )
    //         .unwrap(),
    //         buffered_payload.len()
    //     );

    //     let read_back =
    //         read_file_at(&file, mmap_offset, mmap_payload.len(), StatusFlags::empty()).unwrap();
    //     assert_eq!(read_back, mmap_payload);
    // }

    #[ktest]
    fn falloc_allocate_returns_enospc_after_consuming_blocks() {
        clocks::init_for_ktest();

        let f = Ext2FixtureBuilder::new(1, 256)
            .with_free_blocks(2, 2)
            .build()
            .unwrap();
        let file = make_live_file_inode(
            &f.ext2,
            69,
            0,
            0,
            FileFlags::empty(),
            [0; RAW_BLOCK_PTRS_LEN],
        );

        file.fallocate(FallocMode::Allocate, 0, BLOCK_SIZE * 2)
            .unwrap();
        assert_eq!(f.ext2.super_block().free_blocks_count(), 0);
        assert_eq!(file.file_size(), BLOCK_SIZE * 2);

        assert_errno!(
            file.fallocate(FallocMode::Allocate, BLOCK_SIZE * 2, BLOCK_SIZE),
            Errno::ENOSPC
        );
        assert_eq!(f.ext2.super_block().free_blocks_count(), 0);
        assert_eq!(file.file_size(), BLOCK_SIZE * 2);
    }
}
