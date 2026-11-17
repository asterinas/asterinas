// SPDX-License-Identifier: MPL-2.0

//! Immutable runtime inodes backed by checked metadata descriptions.
//!
//! An inode owns its description and only weakly refers to its filesystem.
//! The shared raw backend retains the device. VFS metadata is an immutable
//! snapshot; new image reads still require a live filesystem handle.

mod backend;
pub(super) mod data;
pub(super) mod desc;
pub(super) mod dir;
pub(super) mod fields;
pub(super) mod layout;
pub(super) mod reader;
pub(super) mod symlink;
pub(super) mod time;

use device_id::DeviceId;

use super::{fs::Erofs, io::RawPageBackend, ondisk::super_block::SuperBlock};
use crate::{
    fs::vfs::inode::{Extension, Metadata},
    prelude::{Arc, Weak},
    vm::page_cache::{PageCache, PageCacheBackend},
};

/// An immutable runtime inode sharing its filesystem's physical-page reader.
#[derive(Debug)]
pub(super) struct ErofsInode {
    description: desc::InodeDesc,
    metadata: Metadata,
    extension: Extension,
    fs: Weak<Erofs>,
    data_cache: Arc<PageCache>,
    raw_backend: Arc<RawPageBackend>,
}

impl ErofsInode {
    /// Wraps a checked description from the referenced filesystem snapshot.
    ///
    /// The filesystem loader supplies that provenance; a description's NID
    /// alone cannot prove which device produced it. The raw backend must share
    /// that device snapshot, and metadata must be prepared from the description
    /// and its filesystem configuration. This step performs no I/O.
    pub(super) fn new(
        description: desc::InodeDesc,
        metadata: Metadata,
        fs: Weak<Erofs>,
        raw_backend: Arc<RawPageBackend>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|inode_ref: &Weak<Self>| {
            let capacity_bytes = match description.data_layout() {
                layout::InodeDataLayout::Plain(_)
                | layout::InodeDataLayout::Inline(_)
                | layout::InodeDataLayout::Compressed(_) => {
                    // InodeDesc bounds sizes by i64::MAX on supported 64-bit targets.
                    (description.size_bytes() as usize).next_multiple_of(ostd::mm::PAGE_SIZE)
                }
                layout::InodeDataLayout::Device(_) | layout::InodeDataLayout::NoData => 0,
            };
            let cache_backend: Weak<dyn PageCacheBackend> = inode_ref.clone();
            // A page-cache VMO starts with an empty page array. Aligned capacity
            // and its non-contiguous options make construction infallible.
            let data_cache = Arc::new(
                PageCache::new_with_backend(capacity_bytes, cache_backend)
                    .expect("non-contiguous page-cache VMO construction cannot fail"),
            );
            Self {
                description,
                metadata,
                extension: Extension::new(),
                fs,
                data_cache,
                raw_backend,
            }
        })
    }

    /// Returns the original checked metadata without a mutable projection.
    pub(super) fn description(&self) -> &desc::InodeDesc {
        &self.description
    }

    /// Returns a live filesystem or an explicit expired-back-pointer error.
    pub(super) fn file_system(&self) -> Result<Arc<Erofs>, FileSystemGone> {
        self.fs.upgrade().ok_or(FileSystemGone)
    }
}

/// A runtime inode whose filesystem no longer has any strong owner.
///
/// Kept distinct from image and device failures for error conversion.
#[derive(Debug, Eq, PartialEq)]
pub(super) struct FileSystemGone;

impl ErofsInode {
    /// Returns the immutable VFS snapshot without requiring a live filesystem.
    pub(super) fn vfs_metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// Returns the extension slot shared by all references to this inode.
    pub(super) fn extension(&self) -> &Extension {
        &self.extension
    }

    /// Prepares checked metadata before the inode or filesystem is published.
    ///
    /// The loader supplies the description and device from one filesystem
    /// snapshot. No filesystem weak reference is upgraded during construction.
    pub(super) fn prepare_metadata(
        desc: &desc::InodeDesc,
        super_block: &SuperBlock,
        device_id: DeviceId,
    ) -> ostd::Result<Metadata> {
        use crate::process::{Gid, Uid};

        let size = usize::try_from(desc.size_bytes()).map_err(|_| ostd::Error::Overflow)?;
        let compressed_blocks = match desc.data_layout() {
            layout::InodeDataLayout::Compressed(layout) => layout.compressed_blocks(),
            _ => 0,
        };
        let nr_sectors_allocated = if compressed_blocks != 0 {
            compressed_blocks as usize * (super_block.block_size_bytes() / aster_block::SECTOR_SIZE)
        } else {
            data::checked_cache_capacity(desc.size_bytes())? / aster_block::SECTOR_SIZE
        };
        let nr_hard_links = usize::try_from(desc.nlink()).map_err(|_| ostd::Error::Overflow)?;
        let self_dev_id = match desc.data_layout() {
            layout::InodeDataLayout::Device(id) => Some(id),
            _ => None,
        };
        let time = desc.time();
        Ok(Metadata {
            ino: u64::from(desc.inode_serial()),
            size,
            optimal_block_size: super_block.block_size_bytes(),
            nr_sectors_allocated,
            last_access_at: time.atime(),
            last_modify_at: time.mtime(),
            last_meta_change_at: time.ctime(),
            type_: desc.inode_type(),
            mode: desc.mode(),
            nr_hard_links,
            uid: Uid::new(desc.uid()),
            gid: Gid::new(desc.gid()),
            container_dev_id: device_id,
            self_dev_id,
            birth_at: None,
        })
    }
}
