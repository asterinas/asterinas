// SPDX-License-Identifier: MPL-2.0

//! Ext2 inode implementation.
//!
//! The central type, `Inode`, represents one in-memory ext2 inode and is the
//! concrete inode object exposed to the VFS adapter.
//!
//! # Type aliases
//!
//! - `Iblock` — logical block index within a file (0-based).
//! - `Ext2Bid` — physical block address on the backing device.
//! - `Ext2Ino` — on-disk inode number (ext2 format is 32-bit; the VFS widens
//!   to `u64`).
//!
//! # Data model
//!
//! Inode state is organized as a strict ownership chain:
//!
//! ```text
//! Inode  →  InodeInner  →  InodeDesc  →  RawInode
//! ```
//!
//! - `Inode` — the ref-counted, shareable handle; holds the inode number,
//!   file type, block-group index, a weak filesystem reference, and an optional
//!   xattr sidecar.
//! - `InodeInner` — the `RwMutex`-guarded interior: the dirty descriptor
//!   and the type-specific inode payload.
//! - `InodeDesc` — a decoded, Rust-typed mirror of all on-disk inode fields.
//! - `RawInode` — the 128-byte on-disk layout (`#[repr(C)]`); converted
//!   to/from `InodeDesc` at I/O boundaries.
//!
//! # Submodules
//!
//! The inode implementation is split by inode concern:
//!
//! | Submodule                  | Responsibility                                            |
//! |----------------------------|-----------------------------------------------------------|
//! | `attrs`                    | Metadata: mode, uid, gid, times, xattr                    |
//! | `block_manager`            | Page-cache backend and block-pointer tree management      |
//! | `io_range`                 | Direct-I/O block range planning                           |
//! | `file`                     | Regular-file I/O and allocation                           |
//! | `dir`                      | Directory entry semantics                                 |
//! | `symlink`                  | Symlink target storage                                    |
//! | `sync`                     | Writeback and reclaim                                     |
//!
//! # Locking
//!
//! Within a single inode, `inner` and `xattr` are never held simultaneously;
//! `Xattr` manages its own internal lock and is always accessed outside
//! `inner`. Data-backed inodes additionally nest the block-pointer tree and
//! its indirect-block cache under `inner` in order:
//!
//! ```text
//! Inode::inner → BlockPtrTree → IndirectBlockManager
//! ```
//!
//! When multiple inodes must be write-locked simultaneously, acquire them
//! in ascending `ino` order and coalesce duplicates first.
//!
//! Data-backed inode operations that allocate or free blocks call into
//! filesystem-level methods (`Ext2::alloc_blocks`, `Ext2::free_blocks`). The
//! full cross-layer ordering is:
//!
//! ```text
//! Inode::inner → BlockPtrTree → Ext2::super_block → BlockGroup::metadata
//! ```
//!
//! `BlockGroup::inode_cache` is independent: it is never held while
//! acquiring `super_block` or `metadata`, nor while holding `Inode::inner`.

mod attrs;
mod block_manager;
mod dir;
mod file;
mod io_range;
mod symlink;
mod sync;

use ostd::const_assert;

use self::{
    block_manager::{BlockPtrTree, InodeBlockManager, RawBlockPtrs},
    symlink::FastSymlinkTarget,
};
use super::{fs::Ext2, prelude::*, xattr::Xattr};
use crate::fs::{ext2::utils, file::InodeMode, pipe::Pipe, vfs::inode::Extension};

const MAX_LINK_COUNT: u16 = 32000;
/// Size of the ext2 inode `i_block` byte area used by fast symlinks.
const MAX_FAST_SYMLINK_LEN: usize = size_of::<u32>() * RAW_BLOCK_PTRS_LEN;
/// Number of 32-bit entries in the ext2 inode `i_block` array.
pub(super) const RAW_BLOCK_PTRS_LEN: usize = 15;
/// Logical block index within a file (0-based).
pub(super) type Iblock = u32;
/// Physical block address on the device.
pub(super) type Ext2Bid = u32;
/// On-disk inode number (ext2 format is 32-bit; the VFS widens to u64).
pub(super) type Ext2Ino = u32;

/// Ext2 file permission bits (lower 12 bits of `i_mode`).
#[derive(Clone, Copy, Debug)]
pub struct FilePerm(u16);

impl FilePerm {
    /// Constructs a `FilePerm` from raw permission bits, discarding unknown bits.
    pub(super) fn from_bits_truncate(bits: u16) -> Self {
        Self(bits)
    }

    /// Returns the raw permission bits.
    pub(super) fn bits(self) -> u16 {
        self.0
    }
}

/// An in-memory ext2 inode.
///
/// Each `Inode` corresponds to one on-disk inode identified by a unique
/// inode number (`ino`). It caches the inode descriptor and the payload needed
/// by that inode type, then exposes directory, symlink, and regular-file
/// operations through the VFS `Inode` trait.
///
/// An `Inode` owns the cached descriptor and its type-specific payload. Only
/// data-backed payloads own data pages and block-mapping state; block-group
/// bitmaps belong to the parent `Ext2` and `BlockGroup`. See the module-level
/// documentation, especially the `Locking` section, for the full lock ordering
/// across inode, filesystem, and block-group layers.
pub struct Inode {
    ino: Ext2Ino,
    type_: InodeType,
    inner: RwMutex<InodeInner>,
    block_group_idx: usize,
    fs: Weak<Ext2>,
    xattr: Option<Xattr>,
    pipe: Option<Pipe>,
    extension: Extension,
}

impl Inode {
    /// Creates a new `Inode` and returns it wrapped in `Arc`.
    pub(super) fn new(
        ino: Ext2Ino,
        type_: InodeType,
        inode_desc: Dirty<InodeDesc>,
        block_group_idx: usize,
        fs: Weak<Ext2>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|weak_self: &Weak<Self>| {
            let xattr = match type_ {
                InodeType::Dir | InodeType::File => Some(Xattr::new(
                    inode_desc.file_acl,
                    weak_self.clone(),
                    fs.clone(),
                )),
                _ => None,
            };
            let pipe = match type_ {
                InodeType::NamedPipe => Some(Pipe::new()),
                _ => None,
            };
            Self {
                ino,
                type_,
                inner: RwMutex::new(InodeInner::new(inode_desc, fs.clone())),
                block_group_idx,
                fs,
                xattr,
                pipe,
                extension: Extension::new(),
            }
        })
    }

    /// Returns the ext2 inode number.
    pub(super) fn ino(&self) -> Ext2Ino {
        self.ino
    }

    /// Returns the block group index this inode belongs to.
    pub(super) fn block_group_idx(&self) -> usize {
        self.block_group_idx
    }

    /// Returns the hard-link count.
    pub(super) fn link_count(&self) -> u16 {
        self.inner.read().link_count()
    }

    /// Returns a reference to the owning `Ext2` filesystem.
    pub(super) fn fs(&self) -> Result<Arc<Ext2>> {
        self.fs
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "filesystem already dropped"))
    }

    /// Returns the logical file size in bytes.
    pub(super) fn file_size(&self) -> usize {
        self.inner.read().file_size()
    }

    /// Returns a reference to the VFS extension slot.
    pub(super) fn extension(&self) -> &Extension {
        &self.extension
    }

    /// Returns a reference to the pipe if this is a named-pipe inode.
    pub(in crate::fs::fs_impls::ext2) fn pipe(&self) -> Option<&Pipe> {
        self.pipe.as_ref()
    }

    /// Returns a clone of the data page cache for this inode if it has one.
    pub(super) fn page_cache(&self) -> Option<PageCache> {
        self.inner.read().page_cache_clone()
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

/// Parsed in-memory mirror of an on-disk inode's metadata fields.
///
/// Unlike `RawInode`, fields are decoded into Rust types
/// (e.g., `Duration` for timestamps, `InodeType` for file type).
#[derive(Clone, Copy, Debug)]
pub(super) struct InodeDesc {
    type_: InodeType,
    perm: FilePerm,
    uid: u32,
    gid: u32,
    size: u64,
    atime: Duration,
    ctime: Duration,
    mtime: Duration,
    dtime: Duration,
    link_count: u16,
    sector_count: u32,
    flags: FileFlags,
    file_acl: u32,
    generation: u32,
    block_ptrs: [u32; RAW_BLOCK_PTRS_LEN],
}

impl InodeDesc {
    pub(super) fn new(
        type_: InodeType,
        perm: FilePerm,
        uid: u32,
        gid: u32,
        link_count: u16,
        generation: u32,
        now: Duration,
    ) -> Self {
        Self {
            type_,
            perm,
            uid,
            gid,
            size: 0,
            atime: now,
            ctime: now,
            mtime: now,
            dtime: Duration::ZERO,
            link_count,
            sector_count: 0,
            flags: FileFlags::empty(),
            file_acl: 0,
            generation,
            block_ptrs: [0; RAW_BLOCK_PTRS_LEN],
        }
    }

    /// Returns the inode type stored in this descriptor.
    pub(super) fn type_(&self) -> InodeType {
        self.type_
    }
}

impl TryFrom<&RawInode> for InodeDesc {
    type Error = Error;
    fn try_from(raw: &RawInode) -> Result<Self> {
        if raw.link_count == 0 {
            return_errno_with_message!(Errno::ESTALE, "inode has been deleted");
        }

        let mode = raw.mode;
        let type_ = InodeType::from_raw_mode(mode)?;
        let perm = FilePerm::from_bits_truncate(mode & 0o7777);
        let uid = (raw.uid as u32) | ((raw.uid_high as u32) << 16);
        let gid = (raw.gid as u32) | ((raw.gid_high as u32) << 16);
        let atime = Duration::from_secs(raw.atime as u64);
        let ctime = Duration::from_secs(raw.ctime as u64);
        let mtime = Duration::from_secs(raw.mtime as u64);

        let mut size = raw.size_lo as u64;
        if type_ == InodeType::File {
            size |= (raw.size_high as u64) << 32;
        }

        // Linux stores the nul byte in symlink, so the maximum size is `BLOCK_SIZE - 1`.
        if type_ == InodeType::SymLink && size >= BLOCK_SIZE as u64 {
            return_errno_with_message!(Errno::EUCLEAN, "corrupted symlink on disk");
        }
        if size > i64::MAX as u64 {
            return_errno_with_message!(Errno::EUCLEAN, "corrupted inode on disk");
        }

        let flags = FileFlags::from_bits(raw.flags)
            .ok_or_else(|| Error::with_message(Errno::EIO, "invalid inode flags"))?;
        let raw_block_ptrs = RawBlockPtrs::new(raw.sector_count, raw.block);

        Ok(InodeDesc {
            type_,
            perm,
            uid,
            gid,
            size,
            atime,
            ctime,
            mtime,
            dtime: Duration::from_secs(raw.dtime as u64),
            link_count: raw.link_count,
            sector_count: raw_block_ptrs.sector_count,
            flags,
            file_acl: raw.file_acl,
            generation: raw.generation,
            block_ptrs: raw_block_ptrs.block_ptrs,
        })
    }
}

impl From<&InodeDesc> for RawInode {
    fn from(desc: &InodeDesc) -> Self {
        let mode = (desc.type_ as u16) | (desc.perm.0 & 0o7777);
        let uid = desc.uid as u16;
        let gid = desc.gid as u16;
        let uid_high = (desc.uid >> 16) as u16;
        let gid_high = (desc.gid >> 16) as u16;

        let (size_lo, size_high) = if desc.type_ == InodeType::File {
            (desc.size as u32, (desc.size >> 32) as u32)
        } else {
            (desc.size as u32, 0)
        };

        Self {
            mode,
            uid,
            size_lo,
            atime: utils::duration_to_ext2_secs(desc.atime),
            ctime: utils::duration_to_ext2_secs(desc.ctime),
            mtime: utils::duration_to_ext2_secs(desc.mtime),
            dtime: utils::duration_to_ext2_secs(desc.dtime),
            gid,
            link_count: desc.link_count,
            sector_count: desc.sector_count,
            flags: desc.flags.bits(),
            osd1: 0,
            block: desc.block_ptrs,
            generation: desc.generation,
            file_acl: desc.file_acl,
            size_high,
            faddr: 0,
            frag: 0,
            fsize: 0,
            pad1: 0,
            uid_high,
            gid_high,
            reserved2: 0,
        }
    }
}

/// On-disk inode structure (128 bytes for GOOD_OLD_REV).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct RawInode {
    pub mode: u16,                        // i_mode
    pub uid: u16,                         // i_uid (low 16 bits)
    pub size_lo: u32,                     // i_size
    pub atime: u32,                       // i_atime
    pub ctime: u32,                       // i_ctime
    pub mtime: u32,                       // i_mtime
    pub dtime: u32,                       // i_dtime
    pub gid: u16,                         // i_gid (low 16 bits)
    pub link_count: u16,                  // i_link_count
    pub sector_count: u32,                // i_blocks (512-byte sectors)
    pub flags: u32,                       // i_flags
    pub osd1: u32,                        // osd1.linux1.l_i_reserved1
    pub block: [u32; RAW_BLOCK_PTRS_LEN], // i_block
    pub generation: u32,                  // i_generation
    pub file_acl: u32,                    // i_file_acl
    pub size_high: u32,                   // i_dir_acl (size high)
    pub faddr: u32,                       // i_faddr
    pub frag: u8,                         // osd2.linux2.l_i_frag
    pub fsize: u8,                        // osd2.linux2.l_i_fsize
    pub pad1: u16,                        // osd2.linux2.i_pad1
    pub uid_high: u16,                    // osd2.linux2.l_i_uid_high
    pub gid_high: u16,                    // osd2.linux2.l_i_gid_high
    pub reserved2: u32,                   // osd2.linux2.l_i_reserved2
}

const_assert!(size_of::<RawInode>() == 128);

/// Interior of `Inode` guarded by a single `RwMutex`.
#[derive(Debug)]
struct InodeInner {
    /// Full persistence mirror of the on-disk inode.
    desc: Dirty<InodeDesc>,
    /// Type-specific payload stored in ext2's overloaded `i_block` area.
    payload: InodePayload,
}

/// Type-specific in-memory state backed by ext2 inode payload storage.
#[derive(Debug)]
enum InodePayload {
    /// Regular files, directories, and slow symlinks backed by data blocks.
    DataBacked {
        page_cache: PageCache,
        block_manager: Arc<InodeBlockManager>,
    },
    /// Fast symlink target stored inline in `i_block`.
    FastSymlink { target: FastSymlinkTarget },
    /// Character or block device number encoded in `i_block`.
    Device { device_id: u64 },
    /// Special inode types that do not carry an ext2 inode payload.
    NoPayload,
}

impl InodeInner {
    fn new(inode_desc: Dirty<InodeDesc>, fs: Weak<Ext2>) -> Self {
        let payload = InodePayload::new(&inode_desc, fs);

        Self {
            desc: inode_desc,
            payload,
        }
    }

    fn page_cache(&self) -> &PageCache {
        self.payload
            .page_cache()
            .expect("data-backed inode must have a page cache")
    }

    fn block_manager(&self) -> Result<&Arc<InodeBlockManager>> {
        self.payload.block_manager()
    }

    fn raw_block_ptrs(&self) -> RawBlockPtrs {
        self.payload.raw_block_ptrs(self.desc.file_acl)
    }

    fn page_cache_clone(&self) -> Option<PageCache> {
        self.payload.page_cache().cloned()
    }

    fn resize_page_cache(&mut self, new_size_bytes: usize, old_size_bytes: usize) -> Result<()> {
        let InodePayload::DataBacked {
            page_cache,
            block_manager,
        } = &self.payload
        else {
            return_errno_with_message!(Errno::EINVAL, "inode has no data page cache");
        };
        page_cache.resize(new_size_bytes, old_size_bytes)?;
        // Reset the npages to let the `PageCacheBackend` know the size of page cache.
        block_manager.set_npages(new_size_bytes.div_ceil(PAGE_SIZE));
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

    fn inode_type(&self) -> InodeType {
        self.desc.type_
    }

    fn mode(&self) -> InodeMode {
        InodeMode::from_bits_truncate(self.desc.perm.bits())
    }

    fn set_mode(&mut self, mode: InodeMode) {
        self.desc.perm = FilePerm::from_bits_truncate(mode.bits());
    }

    fn uid(&self) -> u32 {
        self.desc.uid
    }

    fn set_uid(&mut self, uid: u32) {
        self.desc.uid = uid;
    }

    fn gid(&self) -> u32 {
        self.desc.gid
    }

    fn set_gid(&mut self, gid: u32) {
        self.desc.gid = gid;
    }

    fn file_size(&self) -> usize {
        self.desc.size as usize
    }

    fn set_file_size(&mut self, new_size: usize) {
        self.desc.size = new_size as u64;
    }

    fn atime(&self) -> Duration {
        self.desc.atime
    }

    fn set_atime(&mut self, time: Duration) {
        self.desc.atime = time;
    }

    fn mtime(&self) -> Duration {
        self.desc.mtime
    }

    fn set_mtime(&mut self, time: Duration) {
        self.desc.mtime = time;
    }

    fn ctime(&self) -> Duration {
        self.desc.ctime
    }

    fn set_ctime(&mut self, time: Duration) {
        self.desc.ctime = time;
    }

    fn set_mtime_ctime(&mut self, time: Duration) {
        self.set_mtime(time);
        self.set_ctime(time);
    }

    fn set_dtime(&mut self, time: Duration) {
        self.desc.dtime = time;
    }

    fn link_count(&self) -> u16 {
        self.desc.link_count
    }

    fn set_link_count(&mut self, count: u16) {
        self.desc.link_count = count;
    }

    fn inc_link_count(&mut self, delta: u16) {
        debug_assert!(self.desc.link_count <= u16::MAX - delta);
        self.desc.link_count += delta;
    }

    fn dec_link_count(&mut self, delta: u16) {
        debug_assert!(self.desc.link_count >= delta);
        self.desc.link_count = self.desc.link_count.saturating_sub(delta);
    }

    fn remove_flags(&mut self, flags: FileFlags) {
        self.desc.flags.remove(flags);
    }

    fn set_file_acl(&mut self, file_acl: u32) {
        self.desc.file_acl = file_acl;
    }
}

impl InodePayload {
    fn new(inode_desc: &Dirty<InodeDesc>, fs: Weak<Ext2>) -> Self {
        let raw_block_ptrs = RawBlockPtrs::new(inode_desc.sector_count, inode_desc.block_ptrs);
        match inode_desc.type_ {
            InodeType::File | InodeType::Dir => {
                Self::new_data_backed(inode_desc.size as usize, raw_block_ptrs, fs)
            }
            InodeType::SymLink if Self::is_fast_symlink(inode_desc) => Self::FastSymlink {
                target: FastSymlinkTarget::new(inode_desc.block_ptrs),
            },
            InodeType::SymLink => {
                Self::new_data_backed(inode_desc.size as usize, raw_block_ptrs, fs)
            }
            InodeType::CharDevice | InodeType::BlockDevice => Self::Device {
                device_id: raw_block_ptrs.read_device_id(),
            },
            _ => Self::NoPayload,
        }
    }

    fn new_data_backed(size: usize, raw_block_ptrs: RawBlockPtrs, fs: Weak<Ext2>) -> Self {
        let page_cache_size = size.align_up(PAGE_SIZE);
        let page_count = page_cache_size / PAGE_SIZE;
        let block_ptr_tree = BlockPtrTree::new(raw_block_ptrs, fs.clone());
        let block_manager = Arc::new(InodeBlockManager::new(
            block_ptr_tree,
            fs.clone(),
            page_count,
        ));
        let page_cache_backend: Weak<dyn PageCacheBackend> = Arc::downgrade(&block_manager) as _;
        // Keep page-cache capacity aligned with inode size so `npages`/VMO window
        // and on-disk data extent stay consistent from mount time.
        let page_cache = PageCache::new_with_backend(page_cache_size, page_cache_backend)
            .expect("ext2 inode page cache allocation failed");

        Self::DataBacked {
            page_cache,
            block_manager,
        }
    }

    fn is_fast_symlink(inode_desc: &Dirty<InodeDesc>) -> bool {
        let xattr_sectors = Self::xattr_sectors(inode_desc.file_acl);
        inode_desc.type_ == InodeType::SymLink && inode_desc.sector_count == xattr_sectors
    }

    fn xattr_sectors(file_acl: u32) -> u32 {
        if file_acl == 0 {
            0
        } else {
            (BLOCK_SIZE / SECTOR_SIZE) as u32
        }
    }

    fn page_cache(&self) -> Option<&PageCache> {
        match self {
            Self::DataBacked { page_cache, .. } => Some(page_cache),
            _ => None,
        }
    }

    fn block_manager(&self) -> Result<&Arc<InodeBlockManager>> {
        match self {
            Self::DataBacked { block_manager, .. } => Ok(block_manager),
            _ => Err(Error::with_message(
                Errno::EINVAL,
                "inode payload has no block manager",
            )),
        }
    }

    fn raw_block_ptrs(&self, file_acl: u32) -> RawBlockPtrs {
        match self {
            Self::DataBacked { block_manager, .. } => block_manager.raw_block_ptrs(),
            Self::FastSymlink { target } => {
                RawBlockPtrs::new(Self::xattr_sectors(file_acl), target.block_ptrs())
            }
            Self::Device { device_id } => {
                let mut raw_block_ptrs = RawBlockPtrs::new(0, [0; RAW_BLOCK_PTRS_LEN]);
                raw_block_ptrs.write_device_id(*device_id);
                raw_block_ptrs
            }
            Self::NoPayload => RawBlockPtrs::new(0, [0; RAW_BLOCK_PTRS_LEN]),
        }
    }
}

bitflags! {
    struct FileFlags: u32 {
        /// Secure deletion.
        const SECURE_DEL = 1 << 0;
        /// Undelete.
        const UNDELETE = 1 << 1;
        /// Compresses the file.
        const COMPRESS = 1 << 2;
        /// Synchronous updates.
        const SYNC_UPDATE = 1 << 3;
        /// Immutable file.
        const IMMUTABLE = 1 << 4;
        /// Append only.
        const APPEND_ONLY = 1 << 5;
        /// Do not dump file.
        const NO_DUMP = 1 << 6;
        /// Does not update `atime`.
        const NO_ATIME = 1 << 7;
        /// Dirty.
        const DIRTY = 1 << 8;
        /// One or more compressed clusters.
        const COMPRESS_BLK = 1 << 9;
        /// Does not compress.
        const NO_COMPRESS = 1 << 10;
        /// Encrypted file.
        const ENCRYPT = 1 << 11;
        /// Hash-indexed directory.
        const INDEX_DIR = 1 << 12;
        /// AFS directory.
        const IMAGIC = 1 << 13;
        /// Journal file data.
        const JOURNAL_DATA = 1 << 14;
        /// File tail should not be merged.
        const NO_TAIL = 1 << 15;
        /// Dirsync behavior (directories only).
        const DIR_SYNC = 1 << 16;
        /// Top of directory hierarchies.
        const TOP_DIR = 1 << 17;
        /// Reserved for the ext2 library.
        const RESERVED = 1 << 31;
    }
}

#[cfg(ktest)]
mod test {
    use super::*;
    use crate::{
        fs::fs_impls::ext2::test_utils::{self, RawInodeBuilder},
        prelude::*,
    };

    /// Reads a `RawInode` directly from the test fixture's disk image.
    pub(super) fn read_raw_inode_from_disk(f: &test_utils::Ext2Fixture, ino: u32) -> RawInode {
        let nr_inodes_per_group = f.sb.nr_inodes_per_group();
        let group_idx = ((ino - 1) / nr_inodes_per_group) as usize;
        let inode_idx = (ino - 1) % nr_inodes_per_group;
        let inode_size = f.sb.inode_size();
        let offset_bytes = (inode_idx as usize) * inode_size;
        let block_index = offset_bytes / BLOCK_SIZE;
        let offset_in_block = offset_bytes % BLOCK_SIZE;
        let table_block = f.descs[group_idx].inode_table_bid + block_index as u32;
        f.disk
            .segment()
            .read_val(Bid::new(table_block as u64).to_offset() + offset_in_block)
            .unwrap()
    }

    /// Builds a live in-memory file inode (not on disk) for low-level tests.
    pub(super) fn make_live_file_inode(
        ext2: &Arc<Ext2>,
        ino: u32,
        size: usize,
        sector_count: u32,
        flags: FileFlags,
        block_ptrs: [u32; RAW_BLOCK_PTRS_LEN],
    ) -> Arc<Inode> {
        let mut raw = RawInodeBuilder::new(0o100644).build();
        raw.size_lo = size as u32;
        raw.sector_count = sector_count;
        raw.flags = flags.bits();
        raw.block = block_ptrs;
        let desc = InodeDesc::try_from(&raw).unwrap();
        Inode::new(
            ino,
            InodeType::File,
            Dirty::new(desc),
            0,
            Arc::downgrade(ext2),
        )
    }
}
