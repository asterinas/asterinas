// SPDX-License-Identifier: MPL-2.0

//! Symlink read and write for ext2 inodes.
//!
//! Ext2 distinguishes two storage strategies for symbolic link targets:
//!
//! - **Fast symlink** — targets up to 59 bytes are stored inline in the
//!   60-byte raw block-pointer area of the inode (`i_block[0..15]`) without
//!   allocating any data blocks. One byte is reserved for the
//!   Linux-compatible trailing NUL.
//! - **Slow symlink** — longer targets are written to an allocated data block
//!   through the normal page-cache path.

use super::{
    super::Ext2, Inode, InodeInner, InodePayload, MAX_FAST_SYMLINK_LEN, RAW_BLOCK_PTRS_LEN,
    block_manager::RawBlockPtrs,
};
use crate::fs::ext2::{prelude::*, utils};

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

    fn write(&mut self, target: &[u8]) -> Result<()> {
        debug_assert!(target.len() <= MAX_FAST_SYMLINK_LEN);
        self.block_ptrs.as_mut_bytes()[..target.len()].copy_from_slice(target);
        Ok(())
    }

    fn read(&self, len: usize) -> Vec<u8> {
        self.block_ptrs.as_bytes()[..len].to_vec()
    }

    pub(super) fn block_ptrs(&self) -> [u32; RAW_BLOCK_PTRS_LEN] {
        self.block_ptrs
    }
}

impl Inode {
    /// Reads symbolic link target bytes and decodes them as UTF-8.
    pub(in crate::fs::fs_impls::ext2) fn read_link(&self) -> Result<String> {
        if self.type_ != InodeType::SymLink {
            return_errno!(Errno::EINVAL);
        }

        let inner = self.inner.read();
        inner.read_link()
    }

    /// Writes symbolic link target bytes into either fast-inline or slow-page-cache storage.
    pub(in crate::fs::fs_impls::ext2) fn write_link(&self, target: &str) -> Result<()> {
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

impl InodeInner {
    /// Returns whether this symlink uses fast (inline) storage.
    pub(super) fn is_fast_symlink(&self) -> bool {
        matches!(self.payload, InodePayload::FastSymlink { .. })
    }

    fn write_link(&mut self, fs: &Arc<Ext2>, target: &str) -> Result<()> {
        let target_len = target.len();

        // Linux reserves one byte in `i_block` for a trailing NUL.
        if target.len() < MAX_FAST_SYMLINK_LEN {
            // Fast path: store target inline in the block pointer area.
            if let InodePayload::DataBacked { block_manager, .. } = &self.payload {
                block_manager.truncate_to_byte_len(0);
            }
            let mut fast_target = FastSymlinkTarget::new_zeros();
            fast_target.write(target.as_bytes())?;
            self.payload = InodePayload::FastSymlink {
                target: fast_target,
            };
        } else {
            // Slow path: write through the page cache.
            if !matches!(self.payload, InodePayload::DataBacked { .. }) {
                let raw_block_ptrs = RawBlockPtrs::new(
                    InodePayload::xattr_sectors(self.desc.file_acl),
                    [0; RAW_BLOCK_PTRS_LEN],
                );
                self.payload = InodePayload::new_data_backed(
                    self.file_size(),
                    raw_block_ptrs,
                    Arc::downgrade(fs),
                );
            }
            self.prepare_write(fs.as_ref(), 0, target_len)?;
            self.page_cache().write_bytes(0, target.as_bytes())?;
        }

        self.set_file_size(target_len);
        Ok(())
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

        let mut target = vec![0u8; link_size];
        self.page_cache().read_bytes(0, &mut target).map_err(|_| {
            Error::with_message(Errno::EIO, "failed to read symlink target from page cache")
        })?;

        String::from_utf8(target)
            .map_err(|_| Error::with_message(Errno::EIO, "symlink target is not valid UTF-8"))
    }
}
