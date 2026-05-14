// SPDX-License-Identifier: MPL-2.0

//! Ext4 inode representation.

use core::sync::atomic::{AtomicUsize, Ordering};

use inherit_methods_macro::inherit_methods;

use super::{
    dir::{DirEntryHeader, DirEntryItem, DirEntryReader, MAX_FNAME_LEN},
    extent::ExtentReader,
    fs::Ext4,
    prelude::*,
};

/// The root inode number.
const ROOT_INO: u32 = 2;

/// The Ext4 inode.
pub struct Inode {
    ino: u32,
    type_: InodeType,
    block_group_idx: usize,
    inner: RwMutex<InodeInner>,
    fs: Weak<Ext4>,
}

impl Inode {
    pub(super) fn new(ino: u32, block_group_idx: usize, desc: InodeDesc, fs: Weak<Ext4>) -> Arc<Self> {
        Arc::new_cyclic(|_weak_self| Self {
            ino,
            type_: desc.type_,
            block_group_idx,
            inner: RwMutex::new(InodeInner::new(desc, fs.clone())),
            fs,
        })
    }

    pub fn ino(&self) -> u32 {
        self.ino
    }

    pub fn inode_type(&self) -> InodeType {
        self.type_
    }

    pub fn fs(&self) -> Arc<Ext4> {
        self.fs.upgrade().unwrap()
    }

    pub fn file_size(&self) -> usize {
        self.inner.read().desc.size
    }

    pub fn page_cache(&self) -> Arc<Vmo> {
        self.inner.read().page_cache.pages().clone()
    }

    /// Look up an entry in a directory by name.
    pub fn lookup(&self, name: &str) -> Result<Arc<Self>> {
        if name.len() > MAX_FNAME_LEN {
            return_errno!(Errno::ENAMETOOLONG);
        }
        if self.type_ != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let mut inner = self.inner.write();
        let ino = inner
            .find_entry_item(name)
            .map(|entry| entry.ino())
            .ok_or(Error::new(Errno::ENOENT))?;
        drop(inner);
        self.fs().lookup_inode(ino)
    }

    /// Read directory entries starting from the given offset.
    pub fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        if self.type_ != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let inner = self.inner.read();
        let mut dir_entry_reader = DirEntryReader::new(&inner.page_cache, offset);
        let mut iterate_offset = offset;
        for dir_entry in dir_entry_reader.iter_entries() {
            visitor.visit(
                dir_entry.name(),
                dir_entry.ino() as u64,
                dir_entry.type_(),
                dir_entry.record_len(),
            )?;
            iterate_offset += dir_entry.record_len();
        }
        Ok(iterate_offset - offset)
    }

    /// Read file data at the given offset.
    pub fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        if self.type_ != InodeType::File {
            return_errno!(Errno::EISDIR);
        }

        let inner = self.inner.read();
        let (start, read_len) = {
            let file_size = inner.desc.size;
            let start = file_size.min(offset);
            let end = file_size.min(offset + writer.avail());
            (start, end - start)
        };

        inner.page_cache.pages().read(start, writer)?;
        Ok(read_len)
    }
}

impl Debug for Inode {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("Inode")
            .field("ino", &self.ino)
            .field("block_group_idx", &self.block_group_idx)
            .finish()
    }
}

#[inherit_methods(from = "self.desc")]
impl Inode {
    pub fn file_perm(&self) -> FilePerm;
}

struct InodeInner {
    desc: InodeDesc,
    page_cache: PageCache,
    block_mapper: Arc<InodeBlockMapper>,
}

impl InodeInner {
    pub fn new(desc: InodeDesc, fs: Weak<Ext4>) -> Self {
        let num_page_bytes = (desc.blocks_count() as usize) * BLOCK_SIZE;
        let block_mapper = Arc::new(InodeBlockMapper {
            nblocks: AtomicUsize::new(desc.blocks_count() as _),
            extent_data: RwMutex::new(desc.extent_data),
            fs,
        });
        Self {
            page_cache: PageCache::with_capacity(
                num_page_bytes,
                Arc::downgrade(&block_mapper) as _,
            )
            .unwrap(),
            desc,
            block_mapper,
        }
    }

    pub fn contains_entry(&mut self, name: &str) -> bool {
        DirEntryReader::new(&self.page_cache, 0).contains_entry(name)
    }

    pub fn find_entry_item(&mut self, name: &str) -> Option<DirEntryItem> {
        DirEntryReader::new(&self.page_cache, 0).find_entry_item(name)
    }

    pub fn entry_count(&self) -> usize {
        DirEntryReader::new(&self.page_cache, 0).entry_count()
    }
}

/// Maps inode logical blocks to physical blocks using extents.
struct InodeBlockMapper {
    nblocks: AtomicUsize,
    extent_data: RwMutex<[u8; 60]>,
    fs: Weak<Ext4>,
}

impl PageCacheBackend for InodeBlockMapper {
    fn read_page_async(&self, idx: usize, frame: &CachePage) -> Result<BioWaiter> {
        let fs = self.fs.upgrade().ok_or(Error::with_message(Errno::EIO, "fs gone"))?;
        let extent_reader =
            ExtentReader::new(&self.extent_data.read()).map_err(|e| Error::from(e))?;
        let phys_bid = extent_reader
            .find_block(idx as u32, fs.block_device(), fs.block_size())?
            .ok_or(Error::with_message(Errno::EIO, "sparse file not supported yet"))?;

        let bio_segment = BioSegment::new_from_segment(
            Segment::from(frame.clone()).into(),
            BioDirection::FromDevice,
        );
        fs.read_blocks_async(phys_bid, bio_segment)
    }

    fn write_page_async(&self, _idx: usize, _frame: &CachePage) -> Result<BioWaiter> {
        Err(Error::with_message(Errno::EROFS, "read-only filesystem"))
    }

    fn npages(&self) -> usize {
        self.nblocks.load(Ordering::Acquire)
    }
}

/// The in-memory rust inode descriptor for ext4.
#[derive(Clone, Copy, Debug)]
pub(super) struct InodeDesc {
    pub type_: InodeType,
    pub perm: FilePerm,
    pub uid: u32,
    pub gid: u32,
    pub size: usize,
    pub atime: Duration,
    pub ctime: Duration,
    pub mtime: Duration,
    pub hard_links: u16,
    pub sector_count: u32,
    pub flags: FileFlags,
    pub extent_data: [u8; 60],
}

impl InodeDesc {
    /// Returns the number of blocks utilized by this inode.
    pub fn blocks_count(&self) -> u32 {
        if self.type_ == InodeType::SymLink && self.size <= 60 {
            return 0; // Fast symlink
        }
        self.size.div_ceil(BLOCK_SIZE) as u32
    }
}

impl TryFrom<RawInode> for InodeDesc {
    type Error = crate::error::Error;

    fn try_from(inode: RawInode) -> Result<Self> {
        let inode_type = InodeType::from_raw_mode(inode.mode)?;
        Ok(Self {
            type_: inode_type,
            perm: FilePerm::from_raw_mode(inode.mode)?,
            uid: ((inode.os_dependent_2.uid_high as u32) << 16) | inode.uid as u32,
            gid: ((inode.os_dependent_2.gid_high as u32) << 16) | inode.gid as u32,
            size: if inode_type == InodeType::File {
                ((inode.size_high as usize) << 32) | inode.size_low as usize
            } else {
                inode.size_low as usize
            },
            atime: Duration::from(inode.atime),
            ctime: Duration::from(inode.ctime),
            mtime: Duration::from(inode.mtime),
            hard_links: inode.hard_links,
            sector_count: inode.sector_count,
            flags: FileFlags::from_bits(inode.flags)
                .ok_or(Error::with_message(Errno::EINVAL, "invalid file flags"))?,
            extent_data: inode.block_data,
        })
    }
}

/// The on-disk ext4 inode structure.
///
/// Standard inode size is 256 bytes for ext4 (s_inode_size is typically 256).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct RawInode {
    pub mode: u16,
    pub uid: u16,
    pub size_low: u32,
    pub atime: UnixTime,
    pub ctime: UnixTime,
    pub mtime: UnixTime,
    pub dtime: UnixTime,
    pub gid: u16,
    pub hard_links: u16,
    pub sector_count: u32,
    pub flags: u32,
    pub osd1: u32,
    /// Extent tree or block pointers (60 bytes).
    pub block_data: [u8; 60],
    pub generation: u32,
    pub file_acl_lo: u32,
    pub size_high: u32,
    pub obso_faddr: u32,
    pub os_dependent_2: Osd2,
    /// Extra isize fields (beyond 128 bytes).
    pub extra: [u8; 128],
}

/// OS dependent Value 2.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(super) struct Osd2 {
    pub frag_num: u8,
    pub frag_size: u8,
    pad1: u16,
    pub uid_high: u16,
    pub gid_high: u16,
    reserved2: u32,
}

bitflags! {
    pub struct FilePerm: u16 {
        const S_ISUID = 0o4000;
        const S_ISGID = 0o2000;
        const S_ISVTX = 0o1000;
        const S_IRUSR = 0o0400;
        const S_IWUSR = 0o0200;
        const S_IXUSR = 0o0100;
        const S_IRGRP = 0o0040;
        const S_IWGRP = 0o0020;
        const S_IXGRP = 0o0010;
        const S_IROTH = 0o0004;
        const S_IWOTH = 0o0002;
        const S_IXOTH = 0o0001;
    }
}

impl FilePerm {
    pub fn from_raw_mode(mode: u16) -> Result<Self> {
        const PERM_MASK: u16 = 0o7777;
        Self::from_bits(mode & PERM_MASK)
            .ok_or(Error::with_message(Errno::EINVAL, "invalid file perm"))
    }
}

bitflags! {
    pub struct FileFlags: u32 {
        const SECURE_DEL = 1 << 0;
        const UNDELETE = 1 << 1;
        const COMPRESS = 1 << 2;
        const SYNC_UPDATE = 1 << 3;
        const IMMUTABLE = 1 << 4;
        const APPEND_ONLY = 1 << 5;
        const NO_DUMP = 1 << 6;
        const NO_ATIME = 1 << 7;
        const DIRTY = 1 << 8;
        const COMPRESS_BLK = 1 << 9;
        const NO_COMPRESS = 1 << 10;
        const ENCRYPT = 1 << 11;
        const INDEX_DIR = 1 << 12;
        const IMAGIC = 1 << 13;
        const JOURNAL_DATA = 1 << 14;
        const NO_TAIL = 1 << 15;
        const DIR_SYNC = 1 << 16;
        const TOP_DIR = 1 << 17;
        const EXTENTS = 1 << 18;
        const RESERVED = 1 << 31;
    }
}

impl InodeType {
    fn from_raw_mode(mode: u16) -> Result<Self> {
        const TYPE_MASK: u16 = 0o170000;
        match mode & TYPE_MASK {
            0o040000 => Ok(InodeType::Dir),
            0o100000 => Ok(InodeType::File),
            0o120000 => Ok(InodeType::SymLink),
            0o060000 => Ok(InodeType::BlockDevice),
            0o020000 => Ok(InodeType::CharDevice),
            0o010000 => Ok(InodeType::NamedPipe),
            0o140000 => Ok(InodeType::Socket),
            _ => Ok(InodeType::Unknown),
        }
    }
}
