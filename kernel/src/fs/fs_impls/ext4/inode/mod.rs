// SPDX-License-Identifier: MPL-2.0

//! Ext4 inodes: shared type aliases, the on-disk inode, its validated in-memory
//! form, and the buffered write path.
//!
//! An inode decodes its on-disk metadata (type, permissions, owners, size,
//! times, flags) into `InodeDesc` and maps its data through a block-mapping
//! engine (`block_mapping`). Reads, buffered writes, truncation, and attribute
//! changes all go through `Inode`.
//!
//! # Locking
//!
//! Data-backed inodes nest the mapping engine's interior lock under `inner`:
//!
//! ```text
//! Inode::inner → BlockMapping (engine-interior lock)
//! ```
//!
//! Operations that allocate or free blocks call filesystem-level methods
//! (`Ext4::alloc_blocks` / `Ext4::free_blocks`); the full cross-layer lock order
//! is:
//!
//! ```text
//! Inode::inner → BlockMapping (engine-interior lock) → Ext4::super_block → BlockGroup::metadata
//! ```
//!
//! `BlockGroup::inode_cache` is independent: it is never held while acquiring
//! `super_block` or `metadata`, nor while syncing an inode.
//!
//! The xattr's interior lock is likewise independent of `inner`: the two are
//! never held at the same time.
//!
//! `BlockGroup::inode_table_lock` is a leaf: it serializes inode-slot
//! read-modify-writes and only device I/O happens while it is held.

mod attrs;
mod block_mapping;
mod dir;
mod disk;
mod file;
mod symlink;
mod sync;

use self::{block_mapping::BlockMapping, symlink::FastSymlinkTarget};
use super::{fs::Ext4, prelude::*, xattr::Xattr};
use crate::fs::{pipe::Pipe, vfs::inode::Extension};

/// Maximum hard-link count. `link` rejects a request that would exceed it.
pub(super) const MAX_LINK_COUNT: u16 = 32000;

/// Byte capacity of the inline `i_block` area used to store a fast symlink
/// target (`RAW_BLOCK_PTRS_LEN * 4` = 60). A target strictly shorter than this
/// is stored inline; a longer one is stored in an extent-mapped data block.
pub(super) const MAX_FAST_SYMLINK_LEN: usize = RAW_BLOCK_PTRS_LEN * 4;

/// Number of 32-bit slots in `i_block` (60 bytes total).
///
/// In ext4 these 60 bytes hold the inline extent-tree root rather than the
/// direct/indirect block pointers of ext2.
pub(super) const RAW_BLOCK_PTRS_LEN: usize = 15;

/// Logical file block index.
pub(super) type Iblock = u32;

/// Physical block number on the device.
pub(super) type Ext4Bid = u64;

/// Inode number.
pub(super) type Ext4Ino = u32;

/// `i_flags` value marking an inode whose `i_block` holds an extent tree.
#[cfg_attr(not(ktest), expect(dead_code))]
pub(super) const EXTENTS_FL: u32 = 0x0008_0000;

pub use self::disk::FilePerm;
pub(super) use self::disk::{FileFlags, InodeDesc, RawInode, empty_extent_root};

/// A single ext4 inode: shared metadata plus type-specific payload.
pub struct Inode {
    ino: Ext4Ino,
    type_: InodeType,
    inner: RwMutex<InodeInner>,
    block_group_idx: usize,
    fs: Weak<Ext4>,
    /// The extended-attribute block handle; only files and directories
    /// carry attributes. Its interior lock is never held together with
    /// `inner`.
    xattr: Option<Xattr>,
    /// The in-memory pipe backing a named-pipe (FIFO) inode.
    pipe: Option<Pipe>,
    /// The VFS extension slot (flock, POSIX locks, inotify); must exist from
    /// day one or the VFS layer panics on inodes that use these features.
    extension: Extension,
}

impl Inode {
    pub(super) fn new(
        ino: Ext4Ino,
        type_: InodeType,
        desc: Dirty<InodeDesc>,
        block_group_idx: usize,
        fs: Weak<Ext4>,
    ) -> Result<Arc<Self>> {
        let payload = InodePayload::new(&desc, fs.clone())?;
        let pipe = match type_ {
            InodeType::NamedPipe => Some(Pipe::new()),
            _ => None,
        };
        Ok(Arc::new_cyclic(|weak_self: &Weak<Self>| {
            let xattr = match type_ {
                InodeType::Dir | InodeType::File => {
                    Some(Xattr::new(desc.file_acl(), weak_self.clone(), fs.clone()))
                }
                _ => None,
            };
            Self {
                ino,
                type_,
                inner: RwMutex::new(InodeInner { desc, payload }),
                block_group_idx,
                fs,
                xattr,
                pipe,
                extension: Extension::new(),
            }
        }))
    }

    pub(super) fn ino(&self) -> Ext4Ino {
        self.ino
    }

    pub(super) fn block_group_idx(&self) -> usize {
        self.block_group_idx
    }

    pub(super) fn link_count(&self) -> u16 {
        self.inner.read().desc.link_count()
    }

    /// Returns the owning filesystem, or an error if it has been dropped.
    pub(super) fn fs(&self) -> Result<Arc<Ext4>> {
        self.fs
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "filesystem dropped"))
    }

    pub(super) fn size(&self) -> usize {
        self.inner.read().file_size()
    }

    pub(super) fn sector_count(&self) -> u64 {
        self.inner.read().sector_count()
    }

    pub(super) fn extension(&self) -> &Extension {
        &self.extension
    }

    /// Returns the pipe backing a named-pipe (FIFO) inode.
    pub(super) fn pipe(&self) -> Option<&Pipe> {
        self.pipe.as_ref()
    }

    /// Returns a clone of the inode's page cache, if it is data-backed.
    pub(super) fn page_cache(&self) -> Option<PageCache> {
        self.inner.read().page_cache().ok().cloned()
    }
}

impl Drop for Inode {
    fn drop(&mut self) {
        if let Err(err) = self.try_reclaim_deleted_inode() {
            debug!(
                "failed to reclaim deleted inode {} during drop: {:?}",
                self.ino, err
            );
        }
    }
}

struct InodeInner {
    desc: Dirty<InodeDesc>,
    payload: InodePayload,
}

/// Type-specific inode contents.
enum InodePayload {
    /// Regular files and directories: page-cached data mapped through
    /// `BlockMapping`. Also slow (block-backed) symlinks, whose target lives
    /// in a data block.
    DataBacked {
        page_cache: PageCache,
        /// The authoritative block mapping + `i_blocks` accounting, and the
        /// page-cache backend (the page cache holds only a `Weak` to it).
        block_manager: Arc<BlockMapping>,
    },
    /// Fast (inline) symlinks: the target bytes sit in the 60-byte `i_block`
    /// area without any data block, and the `EXTENTS` flag is cleared.
    FastSymlink { target: FastSymlinkTarget },
    /// Character and block devices: the device ID, whose encoding lives in
    /// `i_block` (see [`disk::read_device_id`]).
    Device { device_id: u64 },
    /// Named pipes and sockets, which carry no data payload.
    NoPayload,
}

impl InodeInner {
    fn page_cache(&self) -> Result<&PageCache> {
        match &self.payload {
            InodePayload::DataBacked { page_cache, .. } => Ok(page_cache),
            _ => return_errno_with_message!(Errno::EINVAL, "inode has no page cache"),
        }
    }

    fn block_manager(&self) -> Result<&Arc<BlockMapping>> {
        match &self.payload {
            InodePayload::DataBacked { block_manager, .. } => Ok(block_manager),
            _ => return_errno_with_message!(Errno::EINVAL, "inode has no block manager"),
        }
    }

    /// Returns `i_blocks` (512-byte sectors). For data-backed inodes the block
    /// manager owns the authoritative count; otherwise the descriptor's value.
    fn sector_count(&self) -> u64 {
        match self.block_manager() {
            Ok(bm) => bm.sector_count(),
            Err(_) => self.desc.sector_count(),
        }
    }

    /// Resizes the page cache and keeps the backend's `npages` bound in sync.
    ///
    /// On growth, the file size is published before the VMO grows. On shrink,
    /// the VMO shrinks before the size drops.
    fn resize_page_cache(&mut self, new_size: usize, old_size: usize) -> Result<()> {
        let InodePayload::DataBacked {
            page_cache,
            block_manager,
        } = &self.payload
        else {
            return_errno_with_message!(Errno::EINVAL, "inode has no data page cache");
        };
        page_cache.resize(new_size, old_size)?;
        block_manager.set_npages(new_size.div_ceil(PAGE_SIZE));
        Ok(())
    }

    fn clear_dirty(&mut self) {
        self.desc.clear_dirty();
        if let Ok(block_manager) = self.block_manager() {
            block_manager.clear_dirty();
        }
    }

    fn is_dirty(&self) -> bool {
        self.desc.is_dirty()
            || self
                .block_manager()
                .is_ok_and(|block_manager| block_manager.is_dirty())
    }

    /// Returns the inode's type.
    fn inode_type(&self) -> InodeType {
        self.desc.type_()
    }

    fn file_size(&self) -> usize {
        usize::try_from(self.desc.size()).expect("Asterinas supports 64-bit architectures")
    }

    fn set_file_size(&mut self, new_size: usize) {
        self.desc.set_size(new_size as u64);
    }

    /// Sets the last-metadata-change time. Used by the unlink/rmdir path to bump
    /// the child's ctime when a link is dropped.
    fn set_ctime(&mut self, time: Duration) {
        self.desc.set_ctime(time);
    }

    fn set_mtime_ctime(&mut self, time: Duration) {
        self.desc.set_mtime(time);
        self.desc.set_ctime(time);
    }

    /// Sets the deletion time (`i_dtime`). Used by the reclaim path.
    fn set_dtime(&mut self, time: Duration) {
        self.desc.set_dtime(time);
    }

    /// Returns the current link count.
    fn link_count(&self) -> u16 {
        self.desc.link_count()
    }

    /// Overwrites the link count. Used by the create error path to set it to 0
    /// so `Drop` reclaims the half-built inode.
    fn set_link_count(&mut self, count: u16) {
        self.desc.set_link_count(count);
    }

    /// Adds `delta` to the link count. Used by the create path to bump the
    /// parent directory's count for a new subdirectory's `..` reference.
    fn inc_link_count(&mut self, delta: u16) {
        self.desc.inc_link_count(delta);
    }

    /// Subtracts `delta` from the link count. Used by the unlink/rmdir path to
    /// drop a name's reference; reaching 0 triggers reclaim on the last `Drop`.
    fn dec_link_count(&mut self, delta: u16) {
        self.desc.dec_link_count(delta);
    }

    /// Clears the given inode flags. Used by rename to drop a moved directory's
    /// stale htree `INDEX` flag.
    fn remove_flags(&mut self, flags: FileFlags) {
        self.desc.remove_flags(flags);
    }

    /// Sets the extended-attribute block number (`i_file_acl`).
    fn set_file_acl(&mut self, file_acl: u64) {
        self.desc.set_file_acl(file_acl);
    }
}

impl InodePayload {
    fn new(desc: &InodeDesc, fs: Weak<Ext4>) -> Result<Self> {
        Ok(match desc.type_() {
            InodeType::File | InodeType::Dir => Self::new_data_backed(desc, fs)?,
            // A symlink is fast (inline) when it owns no data blocks (the
            // `is_fast_symlink` rule); otherwise its target lives in a mapped
            // data block. A freshly created symlink (before `write_link`)
            // starts extent-flagged and size 0, so it decodes as `DataBacked`
            // here and `write_link` later flips it to a fast symlink if the
            // target is short.
            InodeType::SymLink => {
                if symlink::is_fast_symlink(desc) {
                    Self::FastSymlink {
                        target: FastSymlinkTarget::new(*desc.raw_block()),
                    }
                } else {
                    Self::new_data_backed(desc, fs)?
                }
            }
            InodeType::CharDevice | InodeType::BlockDevice => Self::Device {
                device_id: disk::read_device_id(desc.raw_block()),
            },
            _ => Self::NoPayload,
        })
    }

    fn new_data_backed(desc: &InodeDesc, fs: Weak<Ext4>) -> Result<Self> {
        let size = usize::try_from(desc.size()).expect("Asterinas supports 64-bit architectures");
        let mapping = BlockMapping::new(desc, fs, size.align_up(PAGE_SIZE) / PAGE_SIZE)?;
        Ok(Self::from_mapping(mapping, size))
    }

    /// Builds a data-backed payload over a fresh, empty mapping, sized to the
    /// inode's current file size (like ext2's fast-to-slow rebuild): the page
    /// cache must cover `[0, size)` because `prepare_write` grows it relative
    /// to the descriptor's size, not the cache's.
    fn new_data_backed_empty(fs: Weak<Ext4>, extent_based: bool, size: usize) -> Result<Self> {
        let npages = size.align_up(PAGE_SIZE) / PAGE_SIZE;
        let mapping = BlockMapping::new_empty(fs, extent_based, npages)?;
        Ok(Self::from_mapping(mapping, size))
    }

    fn from_mapping(mapping: BlockMapping, size: usize) -> Self {
        let page_cache_size = size.align_up(PAGE_SIZE);
        let mapping = Arc::new(mapping);
        let backend: Weak<dyn PageCacheBackend> = Arc::downgrade(&mapping) as _;
        let page_cache = PageCache::new_with_backend(page_cache_size, backend)
            .expect("ext4 inode page cache allocation failed");
        Self::DataBacked {
            page_cache,
            block_manager: mapping,
        }
    }
}
