// SPDX-License-Identifier: MPL-2.0

//! Ext4 inodes: shared type aliases, the on-disk inode, its validated in-memory
//! form, and read access.
//!
//! An inode decodes its on-disk metadata (type, permissions, owners, size,
//! times, flags) into `InodeDesc` and maps its data through an extent tree
//! (`extent_manager`). Reads go through `Inode`; this is a read-only mount, so
//! the write-side methods return `EROFS`.
//!
//! # Locking
//!
//! Data-backed inodes nest the extent tree under `inner`:
//!
//! ```text
//! Inode::inner → ExtentManager::state
//! ```
//!
//! `BlockGroup::inode_cache` is independent of that nesting: repeated lookups of
//! the same inode number return the same in-memory `Inode`, giving concurrent
//! readers a shared object.

use super::{
    checksum::{self, InodeCsumSeed},
    fs::Ext4,
    prelude::*,
};

mod dir;
mod extent_manager;
mod symlink;

use self::{extent_manager::ExtentManager, symlink::FastSymlinkTarget};
use crate::fs::{file::InodeMode, vfs::inode::Extension};

/// Number of 32-bit slots in `i_block` (60 bytes total).
///
/// In ext4 these 60 bytes hold the inline extent-tree root rather than the
/// direct/indirect block pointers of ext2.
pub(super) const RAW_BLOCK_PTRS_LEN: usize = 15;

/// Byte capacity of the inline `i_block` area used to store a fast symlink
/// target (`RAW_BLOCK_PTRS_LEN * 4` = 60). A target strictly shorter than this
/// is stored inline (one byte is reserved for the Linux trailing NUL); a longer
/// one is stored in an extent-mapped data block (a slow symlink).
pub(super) const MAX_FAST_SYMLINK_LEN: usize = RAW_BLOCK_PTRS_LEN * 4;

/// Logical (file-relative) block index (Linux `ext4_lblk_t`, 32-bit).
pub(super) type Iblock = u32;

/// Physical block number on the device. 64-bit from day one so enabling the
/// `64BIT` feature later needs no widening (report §3.3); the on-disk extent
/// encodes a 48-bit physical block.
pub(super) type Ext4Bid = u64;

/// Inode number.
pub(super) type Ext4Ino = u32;

/// `i_flags` value marking an inode with inline data (Linux
/// `EXT4_INLINE_DATA_FL`); unsupported in Phase 1.
#[expect(dead_code)]
pub(super) const INLINE_DATA_FL: u32 = 0x1000_0000;

/// File permission bits (the low 12 bits of `i_mode`).
#[derive(Clone, Copy, Debug)]
pub struct FilePerm(u16);

impl FilePerm {
    /// Constructs a `FilePerm` from raw mode bits, keeping only the low 12.
    pub(super) fn from_bits_truncate(bits: u16) -> Self {
        Self(bits & 0o7777)
    }

    /// Returns the raw permission bits.
    pub(super) const fn bits(&self) -> u16 {
        self.0
    }
}

bitflags! {
    /// Inode flags (`i_flags`).
    pub(super) struct FileFlags: u32 {
        const SECURE_DEL = 1 << 0;
        const UNDELETE = 1 << 1;
        const COMPRESS = 1 << 2;
        const SYNC = 1 << 3;
        const IMMUTABLE = 1 << 4;
        const APPEND = 1 << 5;
        const NODUMP = 1 << 6;
        const NOATIME = 1 << 7;
        /// Directory uses an htree hash index.
        const INDEX = 1 << 12;
        /// `i_blocks` is counted in filesystem blocks, not 512-byte sectors.
        const HUGE_FILE = 1 << 18;
        /// `i_block` holds an extent tree (`EXT4_EXTENTS_FL`).
        const EXTENTS = 1 << 19;
        const EA_INODE = 1 << 21;
        /// The inode has inline data.
        const INLINE_DATA = 1 << 28;
    }
}

/// Validated, Rust-typed in-memory inode metadata.
///
/// The raw `i_block` bytes are retained in `block`; they are parsed into an
/// `ExtentTree` when the inode's payload is built (`InodePayload::new`).
#[derive(Clone, Debug)]
pub(super) struct InodeDesc {
    type_: InodeType,
    perm: FilePerm,
    uid: u32,
    gid: u32,
    size: u64,
    atime: Duration,
    ctime: Duration,
    mtime: Duration,
    crtime: Duration,
    link_count: u16,
    /// `i_blocks` in 512-byte sectors (48-bit: low 32 + high 16).
    sector_count: u64,
    flags: FileFlags,
    #[expect(dead_code)]
    file_acl: u64,
    generation: u32,
    /// Raw `i_block` (60 bytes) — the inline extent-tree root.
    block: [u32; RAW_BLOCK_PTRS_LEN],
}

/// Decodes the ext4 special-file device encoding stored in `i_block`: 8-bit
/// major/minor pairs use the old `(major << 8) | minor` form in word 0,
/// anything wider the `new_encode_dev` form in word 1.
fn decode_device_block(block: &[u32; RAW_BLOCK_PTRS_LEN]) -> u64 {
    let (major, minor) = if block[0] != 0 {
        ((block[0] >> 8) & 0xFF, block[0] & 0xFF)
    } else {
        let dev = block[1];
        ((dev & 0xFFF00) >> 8, (dev & 0xFF) | ((dev >> 12) & 0xFFF00))
    };
    device_id::encode_device_numbers(major, minor)
}

impl InodeDesc {
    /// Returns the device id encoded in `i_block`, for character/block device
    /// inodes (`None` for every other type — their `i_block` is not a device
    /// encoding).
    pub(super) fn device_id(&self) -> Option<u64> {
        match self.type_ {
            InodeType::CharDevice | InodeType::BlockDevice => {
                Some(decode_device_block(&self.block))
            }
            _ => None,
        }
    }

    pub(super) const fn type_(&self) -> InodeType {
        self.type_
    }

    pub(super) const fn size(&self) -> u64 {
        self.size
    }

    pub(super) const fn link_count(&self) -> u16 {
        self.link_count
    }

    pub(super) const fn flags(&self) -> FileFlags {
        self.flags
    }

    pub(super) const fn sector_count(&self) -> u64 {
        self.sector_count
    }

    pub(super) const fn perm(&self) -> FilePerm {
        self.perm
    }

    pub(super) const fn uid(&self) -> u32 {
        self.uid
    }

    pub(super) const fn gid(&self) -> u32 {
        self.gid
    }

    pub(super) const fn atime(&self) -> Duration {
        self.atime
    }

    pub(super) const fn mtime(&self) -> Duration {
        self.mtime
    }

    pub(super) const fn ctime(&self) -> Duration {
        self.ctime
    }

    pub(super) const fn crtime(&self) -> Duration {
        self.crtime
    }

    /// Returns the inode generation (`i_generation`).
    pub(super) const fn generation(&self) -> u32 {
        self.generation
    }

    /// Returns the raw `i_block` bytes holding the extent-tree root.
    pub(super) const fn raw_block(&self) -> &[u32; RAW_BLOCK_PTRS_LEN] {
        &self.block
    }

    /// Returns whether this inode's data is mapped by an extent tree.
    pub(super) fn is_extent_based(&self) -> bool {
        self.flags.contains(FileFlags::EXTENTS)
    }
}

/// Decodes an ext4 timestamp from its seconds field and the `*_extra` field.
///
/// The extra field packs a 2-bit epoch (extending seconds past 2038) in its low
/// bits and nanoseconds in the upper bits (report §4.3).
fn decode_time(secs: u32, extra: u32) -> Duration {
    let epoch = (extra & 0x3) as i64;
    let nsec = extra >> 2;
    // Linux `ext4_decode_extra_time`: the base seconds are SIGNED and the
    // 2-bit epoch extends them upward, so epoch 0 spans 1901..2038 and epoch 1
    // continues seamlessly at 2^31. Decoding the base as unsigned misread
    // foreign images' pre-1970 timestamps as far-future (P1 review item).
    // `Duration` cannot express pre-1970 at all; clamp those to the epoch.
    let secs = (secs as i32) as i64 + (epoch << 32);
    Duration::new(u64::try_from(secs).unwrap_or(0), nsec)
}

impl TryFrom<&RawInode> for InodeDesc {
    type Error = Error;

    fn try_from(raw: &RawInode) -> Result<Self> {
        if raw.link_count == 0 {
            return_errno_with_message!(Errno::ESTALE, "inode is not in use");
        }

        let type_ = InodeType::from_raw_mode(raw.mode)
            .map_err(|_| Error::with_message(Errno::EUCLEAN, "invalid inode mode"))?;
        let perm = FilePerm::from_bits_truncate(raw.mode);

        let uid = (raw.uid as u32) | ((raw.uid_high as u32) << 16);
        let gid = (raw.gid as u32) | ((raw.gid_high as u32) << 16);

        let mut size = raw.size_lo as u64;
        if type_ == InodeType::File {
            size |= (raw.size_high as u64) << 32;
        }
        if type_ == InodeType::SymLink && size >= BLOCK_SIZE as u64 {
            return_errno_with_message!(Errno::EUCLEAN, "symlink size too large");
        }
        if size > i64::MAX as u64 {
            return_errno_with_message!(Errno::EUCLEAN, "inode size too large");
        }

        let flags = FileFlags::from_bits_truncate(raw.flags);
        // NB: not rescaled for `EXT4_HUGE_FILE_FL` inodes (known limitation,
        // see `feature.rs`); `st_blocks` may under-report for huge files.
        let sector_count = (raw.sector_count as u64) | ((raw.blocks_high as u64) << 32);
        let file_acl = (raw.file_acl_lo as u64) | ((raw.file_acl_high as u64) << 32);

        Ok(Self {
            type_,
            perm,
            uid,
            gid,
            size,
            atime: decode_time(raw.atime, raw.atime_extra),
            ctime: decode_time(raw.ctime, raw.ctime_extra),
            mtime: decode_time(raw.mtime, raw.mtime_extra),
            crtime: decode_time(raw.crtime, raw.crtime_extra),
            link_count: raw.link_count,
            sector_count,
            flags,
            file_acl,
            generation: raw.generation,
            block: raw.block,
        })
    }
}

/// Byte offset of `i_checksum_lo` in [`RawInode`] (0x7C, inside osd2); the low
/// half of the inode checksum, present in every inode.
const I_CHECKSUM_LO_OFFSET: usize = 0x7C;

/// Byte offset of `i_checksum_hi` in [`RawInode`] (0x82, just past
/// `i_extra_isize`); the high half, present only when the inode carries the
/// extra-size region (`s_inode_size > 128`).
const I_CHECKSUM_HI_OFFSET: usize = 0x82;

impl InodeDesc {
    /// Computes the full 32-bit crc32c of `raw` over the whole `inode_size`, with
    /// both checksum fields treated as zero (Linux `ext4_inode_csum`). The caller
    /// splits it into `i_checksum_lo` (low 16 bits) and, when the inode has the
    /// extra region, `i_checksum_hi` (high 16 bits). `seed` is this inode's
    /// per-inode seed (ino and generation already folded in).
    fn inode_checksum(raw: &RawInode, seed: InodeCsumSeed, inode_size: usize) -> u32 {
        let bytes = raw.as_bytes();
        let mut crc = checksum::crc32c(seed.get(), &bytes[..I_CHECKSUM_LO_OFFSET]);
        crc = checksum::crc32c(crc, &[0u8, 0u8]); // i_checksum_lo
        if inode_size > 128 {
            // The extra region carries i_checksum_hi: checksum the gap between
            // the two fields, then the zeroed hi, then the remainder.
            crc = checksum::crc32c(crc, &bytes[I_CHECKSUM_LO_OFFSET + 2..I_CHECKSUM_HI_OFFSET]);
            crc = checksum::crc32c(crc, &[0u8, 0u8]); // i_checksum_hi
            crc = checksum::crc32c(crc, &bytes[I_CHECKSUM_HI_OFFSET + 2..inode_size]);
        } else {
            crc = checksum::crc32c(crc, &bytes[I_CHECKSUM_LO_OFFSET + 2..inode_size]);
        }
        crc
    }

    /// Verifies `raw`'s stored `i_checksum_lo` (and `i_checksum_hi` when the
    /// inode has the extra region) for a `metadata_csum` volume, at the inode
    /// read boundary. `seed` is this inode's per-inode seed.
    pub(super) fn verify_inode_checksum(
        raw: &RawInode,
        seed: InodeCsumSeed,
        inode_size: usize,
    ) -> Result<()> {
        let crc = Self::inode_checksum(raw, seed, inode_size);
        if raw.checksum_lo != (crc & 0xFFFF) as u16 {
            return_errno_with_message!(Errno::EUCLEAN, "bad inode checksum (lo)");
        }
        if inode_size > 128 && raw.checksum_hi != ((crc >> 16) & 0xFFFF) as u16 {
            return_errno_with_message!(Errno::EUCLEAN, "bad inode checksum (hi)");
        }
        Ok(())
    }
}

const_assert!(size_of::<RawInode>() == 256);

/// The on-disk ext4 inode (256 bytes for the default `s_inode_size`).
///
/// The first 128 bytes match ext2's layout; the trailing fields are ext4's
/// extra-size region (nanosecond timestamps, creation time) followed by space
/// reserved for inline extended attributes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct RawInode {
    pub mode: u16,
    pub uid: u16,
    pub size_lo: u32,
    pub atime: u32,
    pub ctime: u32,
    pub mtime: u32,
    pub dtime: u32,
    pub gid: u16,
    pub link_count: u16,
    /// `i_blocks` low 32 bits (512-byte sectors).
    pub sector_count: u32,
    pub flags: u32,
    pub osd1: u32,
    /// `i_block`: 60 bytes holding the inline extent-tree root.
    pub block: [u32; RAW_BLOCK_PTRS_LEN],
    pub generation: u32,
    pub file_acl_lo: u32,
    pub size_high: u32,
    pub obso_faddr: u32,
    // osd2 (Linux ext4 layout).
    pub blocks_high: u16,
    pub file_acl_high: u16,
    pub uid_high: u16,
    pub gid_high: u16,
    pub checksum_lo: u16,
    pub osd2_reserved: u16,
    // ext4 extra-size region (present when `s_inode_size` > 128).
    pub extra_isize: u16,
    pub checksum_hi: u16,
    pub ctime_extra: u32,
    pub mtime_extra: u32,
    pub atime_extra: u32,
    pub crtime: u32,
    pub crtime_extra: u32,
    pub version_hi: u32,
    pub projid: u32,
    /// Space reserved for inline extended attributes (unused in Phase 1).
    pub tail: InodeTail,
}

/// Padding from the end of the ext4 inode fields (offset 160) to the 256-byte
/// on-disk inode size; in ext4 this holds inline extended attributes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct InodeTail([u8; 96]);

impl Default for InodeTail {
    fn default() -> Self {
        Self([0u8; 96])
    }
}

/// A single ext4 inode: shared metadata plus type-specific payload.
pub struct Inode {
    ino: Ext4Ino,
    type_: InodeType,
    inner: RwMutex<InodeInner>,
    block_group_idx: usize,
    fs: Weak<Ext4>,
    /// The in-memory pipe object backing a named-pipe (FIFO) inode; `None`
    /// for every other type. Created with the inode, like ext2's.
    pipe: Option<crate::fs::pipe::Pipe>,
    /// The VFS extension slot (flock, POSIX locks, inotify); must exist from
    /// day one or the VFS layer panics on inodes that use these features.
    extension: Extension,
}

impl Inode {
    /// Builds a live inode; fails if a data-backed extent root does not parse
    /// (see [`InodePayload::new`]).
    pub(super) fn new(
        ino: Ext4Ino,
        type_: InodeType,
        desc: Dirty<InodeDesc>,
        block_group_idx: usize,
        fs: Weak<Ext4>,
    ) -> Result<Arc<Self>> {
        // With `metadata_csum`, external extent-tree nodes carry a tail checksum
        // seeded per inode (ino + generation folded into the fs seed). Derive it
        // once here; `None` when the feature is off. The read path does not
        // verify external extent-node checksums, so the payload only holds it.
        let csum_seed = fs.upgrade().and_then(|f| {
            let sb = f.super_block();
            sb.has_metadata_csum()
                .then(|| sb.metadata_csum_seed().derive_inode(ino, desc.generation()))
        });
        let payload = InodePayload::new(&desc, fs.clone(), csum_seed)?;
        let pipe = match type_ {
            InodeType::NamedPipe => Some(crate::fs::pipe::Pipe::new()),
            _ => None,
        };
        // The only fallible step (payload parsing) runs before the closure,
        // which just assembles the inode.
        Ok(Arc::new_cyclic(|_self_weak| Self {
            ino,
            type_,
            inner: RwMutex::new(InodeInner { desc, payload }),
            block_group_idx,
            fs,
            pipe,
            extension: Extension::new(),
        }))
    }

    pub(super) fn ino(&self) -> Ext4Ino {
        self.ino
    }

    pub(super) fn inode_type(&self) -> InodeType {
        self.type_
    }

    pub(super) fn size(&self) -> usize {
        self.inner.read().file_size()
    }

    /// Reads file data at `offset` through the inode's page cache.
    pub(super) fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        self.inner.read().read_at(offset, writer)
    }

    pub(super) fn perm(&self) -> FilePerm {
        self.inner.read().desc.perm()
    }

    /// Returns the permission bits as a VFS `InodeMode`.
    pub(super) fn mode(&self) -> InodeMode {
        InodeMode::from_bits_truncate(self.perm().bits() as _)
    }

    pub(super) fn uid(&self) -> u32 {
        self.inner.read().desc.uid()
    }

    pub(super) fn gid(&self) -> u32 {
        self.inner.read().desc.gid()
    }

    pub(super) fn link_count(&self) -> u16 {
        self.inner.read().desc.link_count()
    }

    pub(super) fn sector_count(&self) -> u64 {
        self.inner.read().sector_count()
    }

    pub(super) fn atime(&self) -> Duration {
        self.inner.read().desc.atime()
    }

    pub(super) fn mtime(&self) -> Duration {
        self.inner.read().desc.mtime()
    }

    pub(super) fn ctime(&self) -> Duration {
        self.inner.read().desc.ctime()
    }

    pub(super) fn crtime(&self) -> Duration {
        self.inner.read().desc.crtime()
    }

    #[expect(dead_code)]
    pub(super) fn block_group_idx(&self) -> usize {
        self.block_group_idx
    }

    /// Returns a clone of the inode's page cache, if it is data-backed.
    pub(super) fn page_cache(&self) -> Option<PageCache> {
        self.inner.read().page_cache().ok().cloned()
    }

    /// Returns the pipe object backing a named-pipe (FIFO) inode.
    pub(super) fn pipe(&self) -> Option<&crate::fs::pipe::Pipe> {
        self.pipe.as_ref()
    }

    /// Returns the device id of a character/block device inode (`None`
    /// otherwise).
    pub(super) fn device_id(&self) -> Option<u64> {
        self.inner.read().desc.device_id()
    }

    /// Returns the owning filesystem, or an error if it has been dropped.
    pub(super) fn fs(&self) -> Result<Arc<Ext4>> {
        self.fs
            .upgrade()
            .ok_or_else(|| Error::with_message(Errno::EIO, "filesystem dropped"))
    }

    pub(super) fn extension(&self) -> &Extension {
        &self.extension
    }
}

impl Drop for Inode {
    fn drop(&mut self) {
        // Read-only mount: inode reclaim is a write path and is cut. A read-only
        // volume's link counts never reach 0 in memory, so `Drop` is a natural
        // no-op (the on-disk inode is never freed here).
    }
}

struct InodeInner {
    desc: Dirty<InodeDesc>,
    payload: InodePayload,
}

impl InodeInner {
    fn file_size(&self) -> usize {
        self.desc.size() as usize
    }

    fn page_cache(&self) -> Result<&PageCache> {
        match &self.payload {
            InodePayload::DataBacked { page_cache, .. } => Ok(page_cache),
            _ => return_errno_with_message!(Errno::EINVAL, "inode has no page cache"),
        }
    }

    fn extent_manager(&self) -> Result<&Arc<ExtentManager>> {
        match &self.payload {
            InodePayload::DataBacked { extent_manager, .. } => Ok(extent_manager),
            _ => return_errno_with_message!(Errno::EINVAL, "inode has no block manager"),
        }
    }

    /// Returns `i_blocks` (512-byte sectors). For data-backed inodes the block
    /// manager owns the authoritative count; otherwise the descriptor's value.
    fn sector_count(&self) -> u64 {
        match self.extent_manager() {
            Ok(bm) => bm.sector_count(),
            Err(_) => self.desc.sector_count(),
        }
    }

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
        self.page_cache()?.read(offset, writer)?;
        Ok(read_len)
    }
}

/// Type-specific inode contents.
enum InodePayload {
    /// Regular files and directories: page-cached data mapped by extents. Also
    /// slow (block-backed) symlinks, whose target lives in a data block.
    DataBacked {
        page_cache: PageCache,
        /// The authoritative extent tree + `i_blocks`, and the page-cache
        /// backend (the page cache holds only a `Weak` to it).
        extent_manager: Arc<ExtentManager>,
    },
    /// Fast (inline) symlinks: the target bytes sit in the 60-byte `i_block`
    /// area without any data block, and the `EXTENTS` flag is cleared.
    FastSymlink { target: FastSymlinkTarget },
    /// Inline data (small files stored in the inode); not supported (volumes are
    /// mounted `^inline_data`).
    #[expect(dead_code)]
    Inline,
    /// Devices, FIFOs, and sockets: no in-memory payload — a device node's id is
    /// decoded from the descriptor's `i_block` on demand (`InodeDesc::device_id`).
    NoPayload,
}

impl InodePayload {
    /// Builds the payload for `desc`; fails if a data-backed inode's extent
    /// root does not parse (`ExtentTree::try_new` — the parse-once boundary).
    fn new(desc: &InodeDesc, fs: Weak<Ext4>, csum_seed: Option<InodeCsumSeed>) -> Result<Self> {
        Ok(match desc.type_() {
            // Regular files and directories are both data-backed: page-cached
            // data mapped by an extent tree.
            InodeType::File => Self::new_data_backed(
                desc.size() as usize,
                *desc.raw_block(),
                desc.sector_count(),
                fs,
                csum_seed,
            )?,
            InodeType::Dir => Self::new_data_backed(
                desc.size() as usize,
                *desc.raw_block(),
                desc.sector_count(),
                fs,
                csum_seed,
            )?,
            // A symlink is fast (inline) when it is not extent-based and its
            // target fits strictly within the 60-byte `i_block` area (one byte
            // is reserved for the Linux-compatible trailing NUL). Otherwise it
            // is a slow, extent-mapped data block.
            InodeType::SymLink => {
                let size = desc.size() as usize;
                if !desc.is_extent_based() && size < MAX_FAST_SYMLINK_LEN {
                    Self::FastSymlink {
                        target: FastSymlinkTarget::new(*desc.raw_block()),
                    }
                } else {
                    Self::new_data_backed(
                        size,
                        *desc.raw_block(),
                        desc.sector_count(),
                        fs,
                        csum_seed,
                    )?
                }
            }
            // Devices, FIFOs, and sockets carry no payload (a device id lives in
            // the descriptor's `i_block`, decoded on demand).
            _ => Self::NoPayload,
        })
    }

    fn new_data_backed(
        size: usize,
        root: [u32; RAW_BLOCK_PTRS_LEN],
        sector_count: u64,
        fs: Weak<Ext4>,
        _csum_seed: Option<InodeCsumSeed>,
    ) -> Result<Self> {
        let page_cache_size = size.align_up(PAGE_SIZE);
        let page_count = page_cache_size / PAGE_SIZE;
        let extent_manager = Arc::new(ExtentManager::try_new(root, sector_count, fs, page_count)?);
        let backend: Weak<dyn PageCacheBackend> = Arc::downgrade(&extent_manager) as _;
        let page_cache = PageCache::new_with_backend(page_cache_size, backend)
            .expect("ext4 inode page cache allocation failed");
        Ok(Self::DataBacked {
            page_cache,
            extent_manager,
        })
    }
}

#[cfg(ktest)]
mod tests {
    use ostd::prelude::*;

    use super::*;

    /// Builds a raw root-directory inode: one data block via an (unparsed here)
    /// extent root, link count 2, `EXT4_EXTENTS_FL` set.
    fn raw_root_dir() -> RawInode {
        RawInode {
            mode: 0o040755, // S_IFDIR | 0755
            size_lo: BLOCK_SIZE as u32,
            link_count: 2,
            sector_count: (BLOCK_SIZE / SECTOR_SIZE) as u32,
            flags: FileFlags::EXTENTS.bits(),
            extra_isize: 32,
            ..Default::default()
        }
    }

    /// An inode stamped with its crc32c `i_checksum_lo`/`i_checksum_hi` verifies;
    /// a body change, a wrong inode number, or a wrong generation each fail with
    /// `EUCLEAN`. The seed folds in the inode number and generation, so identical
    /// bytes at a different inode number do not verify.
    #[ktest]
    fn inode_checksum_round_trip() {
        const INODE_SIZE: usize = 256;
        let fs_seed = 0xFEED_BEEF;
        let ino: Ext4Ino = 12;
        let mut raw = raw_root_dir();
        raw.generation = 0x55AA;
        let iseed = checksum::FsCsumSeed::new(fs_seed).derive_inode(ino, raw.generation);

        let crc = InodeDesc::inode_checksum(&raw, iseed, INODE_SIZE);
        raw.checksum_lo = (crc & 0xFFFF) as u16;
        raw.checksum_hi = ((crc >> 16) & 0xFFFF) as u16;
        InodeDesc::verify_inode_checksum(&raw, iseed, INODE_SIZE).unwrap();

        // Wrong inode number / generation are folded into the seed.
        let wrong_ino_seed =
            checksum::FsCsumSeed::new(fs_seed).derive_inode(ino + 1, raw.generation);
        assert_eq!(
            InodeDesc::verify_inode_checksum(&raw, wrong_ino_seed, INODE_SIZE)
                .unwrap_err()
                .error(),
            Errno::EUCLEAN
        );
        // A regenerated inode changes the covered body (its `generation` field),
        // so it no longer verifies against the original seed.
        let mut regen = raw;
        regen.generation = 0x55AB;
        assert!(InodeDesc::verify_inode_checksum(&regen, iseed, INODE_SIZE).is_err());

        // Corrupted body (the covered range excludes the checksum fields).
        let mut bad = raw;
        bad.size_lo += 1;
        assert!(InodeDesc::verify_inode_checksum(&bad, iseed, INODE_SIZE).is_err());

        // Storing the checksum back does not change the covered result.
        let recheck = InodeDesc::inode_checksum(&raw, iseed, INODE_SIZE);
        assert_eq!(recheck, crc);
    }

    #[ktest]
    fn decode_root_dir_inode() {
        let raw = raw_root_dir();
        let desc = InodeDesc::try_from(&raw).unwrap();
        assert_eq!(desc.type_(), InodeType::Dir);
        assert_eq!(desc.size(), BLOCK_SIZE as u64);
        assert_eq!(desc.link_count(), 2);
        assert!(desc.is_extent_based());
        assert_eq!(desc.sector_count(), (BLOCK_SIZE / SECTOR_SIZE) as u64);
    }

    #[ktest]
    fn decode_combines_uid_gid_high() {
        let mut raw = raw_root_dir();
        raw.uid = 0x1111;
        raw.uid_high = 0x2222;
        raw.gid = 0x3333;
        raw.gid_high = 0x4444;
        let desc = InodeDesc::try_from(&raw).unwrap();
        assert_eq!(desc.uid, 0x2222_1111);
        assert_eq!(desc.gid, 0x4444_3333);
    }

    #[ktest]
    fn decode_combines_size_high_for_file() {
        let mut raw = raw_root_dir();
        raw.mode = 0o100644; // S_IFREG | 0644
        raw.link_count = 1;
        raw.size_lo = 0x0000_1000;
        raw.size_high = 0x0000_0001; // 4 GiB + 4 KiB
        let desc = InodeDesc::try_from(&raw).unwrap();
        assert_eq!(desc.type_(), InodeType::File);
        assert_eq!(desc.size(), (1u64 << 32) | 0x1000);
    }

    #[ktest]
    fn decode_nanosecond_time() {
        let mut raw = raw_root_dir();
        raw.mtime = 1000;
        raw.mtime_extra = 500 << 2; // nsec=500, epoch=0
        let desc = InodeDesc::try_from(&raw).unwrap();
        assert_eq!(desc.mtime, Duration::new(1000, 500));
    }

    #[ktest]
    fn reject_unused_inode() {
        let mut raw = raw_root_dir();
        raw.link_count = 0;
        assert!(InodeDesc::try_from(&raw).is_err());
    }
}

#[cfg(ktest)]
mod read_tests {
    use ostd::prelude::*;

    use super::{
        super::test_utils::{Ext4FixtureBuilder, make_unwritten_file_inode},
        *,
    };

    const FILE_INO: u32 = 11;

    /// A preallocated (unwritten) extent reads back as zeros end-to-end through
    /// `Inode::read_at`: the page-cache read path serves an unwritten mapping as
    /// `BioStatus::Zeros` with no device I/O (the Unwritten-first read half). The
    /// blocks count toward `i_blocks` but hold no committed data yet.
    #[ktest]
    fn read_unwritten_extent_reads_as_zeros() {
        let len = 4u16;
        let f = Ext4FixtureBuilder::new(2048, 256, 2048).build().unwrap();
        // A file with a single 4-block unwritten (preallocated) extent at pblock
        // 200 and logical size 4 blocks.
        f.write_raw_inode(
            FILE_INO,
            &make_unwritten_file_inode(200, len, (len as u32) * BLOCK_SIZE as u32),
        );

        let inode = f.ext4.read_inode(FILE_INO).unwrap();
        assert_eq!(inode.size(), len as usize * BLOCK_SIZE);
        assert_eq!(
            inode.sector_count(),
            len as u64 * (BLOCK_SIZE / SECTOR_SIZE) as u64
        );

        // The whole extent reads as zeros while unwritten — the pre-filled buffer
        // is fully overwritten with zeros, proving the read did not skip it.
        let total = len as usize * BLOCK_SIZE;
        let mut buf = vec![0xAAu8; total];
        let mut writer = VmWriter::from(buf.as_mut_slice()).to_fallible();
        let read = inode.read_at(0, &mut writer).unwrap();
        assert_eq!(read, total);
        assert_eq!(buf, vec![0u8; total]);
    }
}
