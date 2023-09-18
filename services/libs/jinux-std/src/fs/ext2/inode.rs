use super::dir::{DirEntry, DirEntryReader, DirEntryWriter};
use super::fs::Ext2;
use super::prelude::*;

use core::cmp::Ordering;
use core::time::Duration;

/// Indirect pointer to blocks.
pub const INDIRECT: u32 = 12;
/// Doubly indirect pointer to blocks.
pub const DB_INDIRECT: u32 = INDIRECT + 1;
/// Trebly indirect pointer to blocks.
pub const TB_INDIRECT: u32 = DB_INDIRECT + 1;
/// Max length of file name.
pub const MAX_FNAME_LEN: usize = 255;
/// Max path length of the fast symlink.
pub const FAST_SYMLINK_MAX_LEN: usize = 60;

/// The ext2 inode in memory.
pub struct Ext2Inode {
    ino: u32,
    block_group_idx: u32,
    raw_inode: RwLock<Dirty<RawInode>>,
    page_cache: PageCache,
    fs: Arc<Ext2>,
}

impl Ext2Inode {
    pub(super) fn new(ino: u32, raw_inode: Dirty<RawInode>, fs: Arc<Ext2>) -> Arc<Self> {
        let page_cache_size = raw_inode.page_cache_size();
        Arc::new_cyclic(|weak_self| Self {
            ino,
            block_group_idx: fs.block_group_idx_of_ino(ino),
            raw_inode: RwLock::new(raw_inode),
            page_cache: PageCache::new(page_cache_size, weak_self.clone() as _).unwrap(),
            fs,
        })
    }

    pub fn ino(&self) -> u32 {
        self.ino
    }

    pub(super) fn block_group_idx(&self) -> u32 {
        self.block_group_idx
    }

    pub fn fs(&self) -> Arc<Ext2> {
        self.fs.clone()
    }

    pub fn file_type(&self) -> FileType {
        self.raw_inode.read().file_type()
    }

    pub fn file_perm(&self) -> FilePerm {
        self.raw_inode.read().file_perm()
    }

    pub fn set_file_perm(&self, perm: FilePerm) {
        self.raw_inode.write().set_file_perm(perm);
    }

    pub fn uid(&self) -> u32 {
        self.raw_inode.read().uid()
    }

    pub fn gid(&self) -> u32 {
        self.raw_inode.read().gid()
    }

    pub fn file_size(&self) -> usize {
        self.raw_inode.read().file_size()
    }

    pub fn page_cache_size(&self) -> usize {
        self.raw_inode.read().page_cache_size()
    }

    pub fn file_flags(&self) -> FileFlags {
        FileFlags::from_bits_truncate(self.raw_inode.read().flags)
    }

    pub fn hard_links(&self) -> u16 {
        self.raw_inode.read().hard_links
    }

    pub(super) fn inc_hard_links(&self) {
        let mut raw_inode = self.raw_inode.write();
        raw_inode.hard_links += 1;
    }

    pub(super) fn dec_hard_links(&self) {
        let mut raw_inode = self.raw_inode.write();
        debug_assert!(raw_inode.hard_links > 0);
        raw_inode.hard_links -= 1;
    }

    pub fn blocks_count(&self) -> u32 {
        self.raw_inode.read().blocks_count
    }

    pub fn acl(&self) -> u32 {
        self.raw_inode.read().acl()
    }

    pub fn atime(&self) -> Duration {
        Duration::from_secs(self.raw_inode.read().atime as _)
    }

    pub fn set_atime(&self, time: Duration) {
        self.raw_inode.write().atime = time.as_secs() as _;
    }

    pub fn mtime(&self) -> Duration {
        Duration::from_secs(self.raw_inode.read().mtime as _)
    }

    pub fn set_mtime(&self, time: Duration) {
        self.raw_inode.write().mtime = time.as_secs() as _;
    }

    pub fn ctime(&self) -> Duration {
        Duration::from_secs(self.raw_inode.read().ctime as _)
    }

    pub fn page_cache(&self) -> &PageCache {
        &self.page_cache
    }

    pub fn device_id(&self) -> u64 {
        match self.file_type() {
            FileType::Block | FileType::Char => {
                let mut device_id: u64 = 0;
                device_id.as_bytes_mut().copy_from_slice(
                    &self.raw_inode.read().data.as_bytes()[..core::mem::size_of::<u64>()],
                );
                device_id
            }
            _ => 0,
        }
    }
}

impl Ext2Inode {
    pub fn read_block(&self, bid: BlockId, block: &mut BioBuf) -> Result<()> {
        let bid: u32 = bid.into();
        if bid >= self.raw_inode.read().blocks_count {
            return Err(Error::InvalidParam);
        }
        debug_assert!(bid < INDIRECT);
        let device_bid = BlockId::new(self.raw_inode.read().data[bid as usize]);
        debug_assert!(block.len() == BLOCK_SIZE);
        self.fs.block_device().read_block(device_bid, block)?;
        Ok(())
    }

    pub fn write_block(&self, bid: BlockId, block: &BioBuf) -> Result<()> {
        let bid: u32 = bid.into();
        if bid >= self.raw_inode.read().blocks_count {
            return Err(Error::InvalidParam);
        }
        debug_assert!(bid < INDIRECT);
        let device_bid = BlockId::new(self.raw_inode.read().data[bid as usize]);
        debug_assert!(block.len() == BLOCK_SIZE);
        self.fs.block_device().write_block(device_bid, block)?;
        Ok(())
    }

    pub fn create(
        &self,
        name: &str,
        file_type: FileType,
        file_perm: FilePerm,
    ) -> Result<Arc<Self>> {
        if self.file_type() != FileType::Dir {
            return Err(Error::NotDir);
        }
        if name.len() > MAX_FNAME_LEN {
            return Err(Error::NameTooLong);
        }
        if self.get_inode_ino(name).is_some() {
            return Err(Error::Exist);
        }

        let inode = self
            .fs
            .new_inode(self.block_group_idx, file_type, file_perm)?;
        if let Err(e) = inode.init(self.ino) {
            self.fs
                .free_inode(inode.ino, file_type == FileType::Dir)
                .unwrap();
            return Err(e);
        }
        let new_entry = DirEntry::new(inode.ino, name, file_type);
        self.append_dir_entry(new_entry)?;
        if file_type == FileType::Dir {
            //for ..
            self.inc_hard_links();
        }

        Ok(inode)
    }

    pub fn write_device_id(&self, device_id: u64) -> Result<()> {
        let file_type = self.file_type();
        if file_type != FileType::Block && file_type != FileType::Char {
            return Err(Error::IsDir);
        }
        let mut raw_inode = self.raw_inode.write();
        raw_inode.data.as_bytes_mut()[..core::mem::size_of::<u64>()]
            .copy_from_slice(device_id.as_bytes());
        Ok(())
    }

    pub fn lookup(&self, name: &str) -> Result<Arc<Self>> {
        if self.file_type() != FileType::Dir {
            return Err(Error::NotDir);
        }
        if name.len() > MAX_FNAME_LEN {
            return Err(Error::NameTooLong);
        }
        self.get_inode(name)
    }

    pub fn link(&self, inode: &Ext2Inode, name: &str) -> Result<()> {
        if self.file_type() != FileType::Dir {
            return Err(Error::NotDir);
        }
        if inode.file_type() == FileType::Dir {
            return Err(Error::IsDir);
        }
        if name.len() > MAX_FNAME_LEN {
            return Err(Error::NameTooLong);
        }
        if self.get_inode_ino(name).is_some() {
            return Err(Error::Exist);
        }

        let new_entry = DirEntry::new(inode.ino(), name, inode.file_type());
        self.append_dir_entry(new_entry)?;
        inode.inc_hard_links();
        Ok(())
    }

    pub fn unlink(&self, name: &str) -> Result<()> {
        if self.file_type() != FileType::Dir {
            return Err(Error::NotDir);
        }
        if name == "." || name == ".." {
            return Err(Error::IsDir);
        }
        if name.len() > MAX_FNAME_LEN {
            return Err(Error::NameTooLong);
        }

        let inode = self.get_inode(name)?;
        if inode.file_type() == FileType::Dir {
            return Err(Error::IsDir);
        }
        self.remove_dir_entry(name)?;
        self.remove_inode(&inode);
        Ok(())
    }

    pub fn rmdir(&self, name: &str) -> Result<()> {
        if self.file_type() != FileType::Dir {
            return Err(Error::NotDir);
        }
        if name == "." || name == ".." {
            return Err(Error::IsDir);
        }
        if name.len() > MAX_FNAME_LEN {
            return Err(Error::NameTooLong);
        }

        let inode = self.get_inode(name)?;
        if inode.file_type() != FileType::Dir {
            return Err(Error::NotDir);
        }
        if DirEntryReader::new(&inode.page_cache, 0).count() > 2 {
            return Err(Error::DirNotEmpty);
        }
        self.remove_dir_entry(name)?;
        self.remove_inode(&inode);
        Ok(())
    }

    pub fn rename(&self, old_name: &str, target: &Ext2Inode, new_name: &str) -> Result<()> {
        if self.file_type() != FileType::Dir || target.file_type() != FileType::Dir {
            return Err(Error::NotDir);
        }
        if old_name == "." || old_name == ".." || new_name == "." || new_name == ".." {
            return Err(Error::IsDir);
        }
        if new_name.len() > MAX_FNAME_LEN || new_name.len() > MAX_FNAME_LEN {
            return Err(Error::NameTooLong);
        }

        let src_inode = self.get_inode(old_name)?;
        let replace_inode = if let Ok(dst_inode) = target.get_inode(new_name) {
            if src_inode.ino() == dst_inode.ino() {
                // Same inode, do nothing
                return Ok(());
            }
            match (src_inode.file_type(), dst_inode.file_type()) {
                (FileType::Dir, FileType::Dir) => {
                    if DirEntryReader::new(&dst_inode.page_cache, 0).count() > 2 {
                        return Err(Error::DirNotEmpty);
                    }
                }
                (FileType::Dir, _) => {
                    return Err(Error::NotDir);
                }
                (_, FileType::Dir) => {
                    return Err(Error::IsDir);
                }
                _ => {}
            }
            Some(dst_inode)
        } else {
            None
        };

        if self.ino() == target.ino() {
            if let Some(replace_inode) = replace_inode {
                self.remove_dir_entry(new_name)?;
                self.remove_inode(&replace_inode);
            }
            self.rename_dir_entry(old_name, new_name)?;
        } else {
            self.remove_dir_entry(old_name)?;
            if let Some(replace_inode) = replace_inode {
                target.remove_dir_entry(new_name)?;
                target.remove_inode(&replace_inode);
            }
            let new_entry = DirEntry::new(src_inode.ino(), new_name, src_inode.file_type());
            target.append_dir_entry(new_entry)?;
            if src_inode.file_type() == FileType::Dir {
                src_inode.set_parent_ino(target.ino())?;
                self.dec_hard_links();
                target.inc_hard_links();
            }
        }

        Ok(())
    }

    pub fn dir_entry_reader(&self, from_offset: usize) -> Result<DirEntryReader> {
        if self.file_type() != FileType::Dir {
            return Err(Error::NotDir);
        }

        Ok(DirEntryReader::new(&self.page_cache, from_offset))
    }

    pub fn write_link(&self, target: &str) -> Result<()> {
        if self.file_type() != FileType::Symlink {
            return Err(Error::IsDir);
        }

        if target.len() <= FAST_SYMLINK_MAX_LEN {
            let mut raw_inode = self.raw_inode.write();
            raw_inode.data.as_bytes_mut()[..target.len()].copy_from_slice(target.as_bytes());
        } else {
            self.page_cache
                .resize(target.len())
                .map_err(|_| Error::NoMemory)?;
            self.page_cache.pages().write_bytes(0, target.as_bytes())?;
        }

        self.resize(target.len())?;
        Ok(())
    }

    pub fn read_link(&self) -> Result<String> {
        if self.file_type() != FileType::Symlink {
            return Err(Error::IsDir);
        }

        let symlink = {
            let file_size = self.file_size();
            let mut symlink = vec![0u8; file_size];

            if file_size <= FAST_SYMLINK_MAX_LEN {
                symlink.copy_from_slice(&self.raw_inode.read().data.as_bytes()[..file_size]);
            } else {
                self.page_cache
                    .pages()
                    .read_bytes(0, symlink.as_mut_slice())?;
            }

            String::from_utf8(symlink).map_err(|_| Error::NotFound)?
        };

        Ok(symlink)
    }

    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        if self.file_type() != FileType::File {
            return Err(Error::IsDir);
        }

        let (offset, read_len) = {
            let file_size = self.file_size();
            let start = file_size.min(offset);
            let end = file_size.min(offset + buf.len());
            (start, end - start)
        };
        self.page_cache
            .pages()
            .read_bytes(offset, &mut buf[..read_len])?;

        Ok(read_len)
    }

    pub fn read_direct_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        if self.file_type() != FileType::File {
            return Err(Error::IsDir);
        }

        let (offset, read_len) = {
            let file_size = self.file_size();
            let start = file_size.min(offset);
            let end = file_size.min(offset + buf.len());
            (start, end - start)
        };

        let mut bio = Bio::from_bytes_mut_at(&mut buf[..read_len], offset);
        for bio_buf_des in bio.bio_bufs_mut().iter_mut() {
            let bid: u32 = bio_buf_des.bid().into();
            debug_assert!(bid < INDIRECT);
            let device_bid = BlockId::new(self.raw_inode.read().data[bid as usize]);
            bio_buf_des.set_bid(device_bid);
        }

        let num_processed = self.fs.block_device().submit_bio(&mut bio)?;

        let mut read_len = 0;
        for i in 0..num_processed {
            read_len += bio.bio_bufs()[i].buf().len();
        }
        Ok(read_len)
    }

    pub fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        if self.file_type() != FileType::File {
            return Err(Error::IsDir);
        }

        let file_size = self.file_size();
        let new_size = offset + buf.len();
        if new_size > file_size {
            self.page_cache
                .resize(new_size)
                .map_err(|_| Error::NoMemory)?;
        }
        self.page_cache.pages().write_bytes(offset, buf)?;
        if new_size > file_size {
            self.resize(new_size)?;
        }

        Ok(buf.len())
    }

    pub fn write_direct_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        if self.file_type() != FileType::File {
            return Err(Error::IsDir);
        }

        let end_offset = offset + buf.len();
        if end_offset > self.file_size() {
            self.resize(end_offset)?;
        }

        let mut bio = Bio::from_bytes_at(buf, offset);
        for bio_buf_des in bio.bio_bufs_mut().iter_mut() {
            let bid: u32 = bio_buf_des.bid().into();
            debug_assert!(bid < INDIRECT);
            let device_bid = BlockId::new(self.raw_inode.read().data[bid as usize]);
            bio_buf_des.set_bid(device_bid);
        }

        let num_processed = self.fs.block_device().submit_bio(&mut bio)?;

        let mut write_len = 0;
        for i in 0..num_processed {
            write_len += bio.bio_bufs()[i].buf().len();
        }
        Ok(write_len)
    }

    pub fn resize(&self, len: usize) -> Result<()> {
        let file_type = self.file_type();
        let blocks = if file_type == FileType::Symlink && len <= FAST_SYMLINK_MAX_LEN {
            0
        } else {
            len.div_ceil(BLOCK_SIZE) as u32
        };

        let mut raw_inode = self.raw_inode.write();
        let old_blocks = raw_inode.blocks_count;
        match blocks.cmp(&old_blocks) {
            Ordering::Greater => {
                // Allocate blocks
                for file_bid in old_blocks..blocks {
                    debug_assert!(file_bid < INDIRECT);
                    let device_bid = self.fs.alloc_block(self.block_group_idx)?;
                    raw_inode.data[file_bid as usize] = device_bid.into();
                }
                raw_inode.blocks_count = blocks;
            }
            Ordering::Equal => (),
            Ordering::Less => {
                // Free blocks
                for file_bid in blocks..old_blocks {
                    debug_assert!(file_bid < INDIRECT);
                    let device_bid = raw_inode.data[file_bid as usize];
                    self.fs.free_block(BlockId::new(device_bid))?;
                }
                raw_inode.blocks_count = blocks;
            }
        }

        raw_inode.set_file_size(len);
        Ok(())
    }

    fn init(&self, dir_ino: u32) -> Result<()> {
        match self.file_type() {
            FileType::Dir => {
                self.init_dir(dir_ino)?;
            }
            _ => {
                // TODO: Reserve serval blocks for regular file ?
            }
        }
        Ok(())
    }

    pub fn sync_all(&self) -> Result<()> {
        self.sync_data()?;
        self.sync_metadata()?;
        Ok(())
    }

    pub fn sync_data(&self) -> Result<()> {
        let page_cache_size = self.page_cache_size();
        self.page_cache
            .evict_range(0..page_cache_size)
            .map_err(|_| Error::IoError)?;
        Ok(())
    }

    pub fn sync_metadata(&self) -> Result<()> {
        if self.raw_inode.read().is_dirty() {
            let mut raw_inode = self.raw_inode.write();
            self.fs.flush_raw_inode(self.ino, &raw_inode)?;
            raw_inode.sync();
        }
        Ok(())
    }

    fn init_dir(&self, parent_ino: u32) -> Result<()> {
        self.append_dir_entry(DirEntry::self_entry(self.ino))?;
        self.append_dir_entry(DirEntry::parent_entry(parent_ino))?;
        self.inc_hard_links();
        Ok(())
    }

    fn get_inode_ino(&self, name: &str) -> Option<u32> {
        self.get_dir_entry(name).map(|(_, entry)| entry.ino())
    }

    fn get_inode(&self, name: &str) -> Result<Arc<Ext2Inode>> {
        let ino = self.get_inode_ino(name).ok_or(Error::NotFound)?;
        self.fs.lookup_inode(ino)
    }

    fn remove_inode(&self, child: &Arc<Ext2Inode>) {
        child.dec_hard_links();
        if child.file_type() == FileType::Dir {
            child.dec_hard_links(); // for .
            self.dec_hard_links(); //  for ..
        }
    }

    fn get_dir_entry(&self, name: &str) -> Option<(usize, DirEntry)> {
        let mut dir_entry_reader = DirEntryReader::new(&self.page_cache, 0);
        dir_entry_reader.find(|(offset, entry)| entry.name() == name)
    }

    fn append_dir_entry(&self, mut new_entry: DirEntry) -> Result<()> {
        let mut dir_entry_reader = DirEntryReader::new(&self.page_cache, 0);
        let Some((offset, mut entry)) =
            dir_entry_reader.find(|(_, entry)| entry.hole_len() >= new_entry.record_len())
        else {
            // Resize and append it at the new block.
            let old_size = self.file_size();
            let new_size = old_size + BLOCK_SIZE;
            self.page_cache
                .resize(new_size)
                .map_err(|_| Error::NoMemory)?;
            let mut dir_entry_writer = DirEntryWriter::new(&self.page_cache, old_size);
            new_entry.set_record_len(BLOCK_SIZE);
            dir_entry_writer.write_dir_entry(&new_entry)?;
            self.resize(new_size)?;
            return Ok(());
        };

        // Write in the hole.
        let mut dir_entry_writer = DirEntryWriter::new(&self.page_cache, offset);
        new_entry.set_record_len(entry.hole_len());
        entry.set_record_len(entry.real_len());
        dir_entry_writer.write_dir_entry(&entry)?;
        dir_entry_writer.write_dir_entry(&new_entry)?;
        Ok(())
    }

    fn remove_dir_entry(&self, name: &str) -> Result<()> {
        let self_entry_record_len = DirEntry::self_entry(self.ino).record_len();
        let reader = DirEntryReader::new(&self.page_cache, 0);
        let next_reader = DirEntryReader::new(&self.page_cache, self_entry_record_len);

        let Some(((pre_offset, mut pre_entry), (offset, entry))) = reader
            .zip(next_reader)
            .find(|((offset, _), (_, dir_entry))| dir_entry.name() == name)
        else {
            return Err(Error::NotFound);
        };

        if DirEntryReader::new(&self.page_cache, offset)
            .next()
            .is_none()
            && pre_offset / BLOCK_SIZE != offset / BLOCK_SIZE
        {
            // Shrink the size.
            let new_size = self.file_size() - BLOCK_SIZE;
            self.page_cache
                .resize(new_size)
                .map_err(|_| Error::IoError)?;
            pre_entry.set_record_len(pre_offset.align_up(BLOCK_SIZE) - pre_offset);
            let mut dir_entry_writer = DirEntryWriter::new(&self.page_cache, pre_offset);
            dir_entry_writer.write_dir_entry(&pre_entry)?;
            self.resize(new_size)?;
        } else {
            // Update the previous entry.
            pre_entry.set_record_len(pre_entry.record_len() + entry.record_len());
            let mut dir_entry_writer = DirEntryWriter::new(&self.page_cache, pre_offset);
            dir_entry_writer.write_dir_entry(&pre_entry)?;
        }

        Ok(())
    }

    fn rename_dir_entry(&self, old_name: &str, new_name: &str) -> Result<()> {
        let (offset, entry) = self.get_dir_entry(old_name).ok_or(Error::NotFound)?;

        let mut new_entry = DirEntry::new(entry.ino(), new_name, entry.type_());
        if new_entry.record_len() <= entry.record_len() {
            // In-place rename.
            new_entry.set_record_len(entry.record_len());
            let mut dir_entry_writer = DirEntryWriter::new(&self.page_cache, offset);
            dir_entry_writer.write_dir_entry(&new_entry)?;
        } else {
            // Move to other position.
            self.remove_dir_entry(old_name)?;
            self.append_dir_entry(new_entry)?;
        }
        Ok(())
    }

    fn set_parent_ino(&self, parent_ino: u32) -> Result<()> {
        let parent_entry = DirEntry::parent_entry(parent_ino);
        let (offset, _) = self.get_dir_entry("..").unwrap();
        let mut dir_entry_writer = DirEntryWriter::new(&self.page_cache, offset);
        dir_entry_writer.write_dir_entry(&parent_entry)?;
        Ok(())
    }

    fn free_blocks(&self) -> Result<()> {
        let raw_inode = self.raw_inode.read();
        debug_assert!(raw_inode.blocks_count < INDIRECT);
        for i in 0..raw_inode.blocks_count {
            let device_bid = raw_inode.data[i as usize];
            self.fs.free_block(BlockId::new(device_bid))?;
        }

        Ok(())
    }
}

impl Debug for Ext2Inode {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("Ext2Inode")
            .field("ino", &self.ino)
            .field("raw_inode", &self.raw_inode.read())
            .field("page_cache", &self.page_cache)
            .finish()
    }
}

impl Drop for Ext2Inode {
    fn drop(&mut self) {
        self.sync_metadata().unwrap();
        if self.hard_links() == 0 {
            self.free_blocks().unwrap();
            self.fs
                .free_inode(self.ino(), self.file_type() == FileType::Dir)
                .unwrap();
        }
    }
}

/// The RawInode is a structure on the disk that represents a file, directory,
/// symbolic link, etc.
/// The inode structure contains pointers to the filesystem blocks which contain the data
/// held in the object and all of the metadata about an object except its name.
///
/// Each block group has an array of inodes (Inode Table) it is responsible for.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, Pod)]
pub(super) struct RawInode {
    /// File mode (type and permissions).
    pub mode: u16,
    /// Low 16 bits of User Id.
    pub uid: u16,
    /// Lower 32 bits of size in bytes.
    pub size_low: u32,
    /// Access time.
    pub atime: u32,
    /// Creation time.
    pub ctime: u32,
    /// Modification time.
    pub mtime: u32,
    /// Deletion time.
    pub dtime: u32,
    /// Low 16 bits of Group Id.
    pub gid: u16,
    /// Hard links count.
    pub hard_links: u16,
    /// Blocks count.
    pub blocks_count: u32,
    /// File Flags.
    pub flags: u32,
    /// OS dependent Value 1.
    reserved1: u32,
    /// Pointers to blocks.
    pub data: [u32; 15],
    /// File version (for NFS).
    pub generation: u32,
    /// In revision 0, this field is reserved.
    /// In revision 1, File ACL.
    pub file_acl: u32,
    /// In revision 0, this field is reserved.
    /// In revision 1, Upper 32 bits of file size (if feature bit set)
    /// if it's a file, Directory ACL if it's a directory.
    pub size_high: u32,
    /// Fragment address.
    pub frag_addr: u32,
    /// OS dependent 2.
    pub os_dependent_2: Osd2,
}

impl RawInode {
    pub fn new(file_type: FileType, file_perm: FilePerm) -> Self {
        Self {
            mode: file_type as u16 | file_perm.bits(),
            hard_links: 1,
            ..Default::default()
        }
    }

    pub fn file_type(&self) -> FileType {
        FileType::from_raw_mode(self.mode)
    }

    pub fn file_perm(&self) -> FilePerm {
        FilePerm::from_raw_mode(self.mode)
    }

    pub fn set_file_perm(&mut self, perm: FilePerm) {
        self.mode = self.file_type() as u16 | perm.bits();
    }

    pub fn file_size(&self) -> usize {
        if self.file_type() == FileType::File {
            (self.size_high as usize) << 32 | self.size_low as usize
        } else {
            self.size_low as usize
        }
    }

    pub fn set_file_size(&mut self, new_size: usize) {
        match self.file_type() {
            FileType::File => {
                self.size_low = new_size as u32;
                self.size_high = (new_size >> 32) as u32;
            }
            _ => {
                self.size_low = new_size as u32;
            }
        }
    }

    pub fn page_cache_size(&self) -> usize {
        (self.blocks_count as usize * BLOCK_SIZE).min(self.file_size())
    }

    pub fn uid(&self) -> u32 {
        (self.os_dependent_2.uid_high as u32) << 16 | self.uid as u32
    }

    pub fn gid(&self) -> u32 {
        (self.os_dependent_2.gid_high as u32) << 16 | self.gid as u32
    }

    pub fn acl(&self) -> u32 {
        if self.file_type() == FileType::File {
            self.file_acl
        } else {
            self.size_high
        }
    }
}

/// OS dependent Value 2
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, Pod)]
pub(super) struct Osd2 {
    /// Fragment number.
    pub frag_num: u8,
    /// Fragment size.
    pub frag_size: u8,
    pad1: u16,
    /// High 16 bits of User Id.
    pub uid_high: u16,
    /// High 16 bits of Group Id.
    pub gid_high: u16,
    reserved2: u32,
}

#[repr(u16)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, TryFromInt)]
pub enum FileType {
    /// FIFO special file
    Fifo = 0o010000,
    /// Character device
    Char = 0o020000,
    /// Directory
    Dir = 0o040000,
    /// Block device
    Block = 0o060000,
    /// Regular file
    File = 0o100000,
    /// Symbolic link
    Symlink = 0o120000,
    /// Socket
    Socket = 0o140000,
}

impl FileType {
    pub fn from_raw_mode(mode: u16) -> Self {
        const TYPE_MASK: u16 = 0o170000;
        Self::try_from(mode & TYPE_MASK).unwrap()
    }
}

bitflags! {
    pub struct FilePerm: u16 {
        /// set-user-ID
        const S_ISUID = 0o4000;
        /// set-group-ID
        const S_ISGID = 0o2000;
        /// sticky bit
        const S_ISVTX = 0o1000;
        /// read by owner
        const S_IRUSR = 0o0400;
        /// write by owner
        const S_IWUSR = 0o0200;
        /// execute/search by owner
        const S_IXUSR = 0o0100;
        /// read by group
        const S_IRGRP = 0o0040;
        /// write by group
        const S_IWGRP = 0o0020;
        /// execute/search by group
        const S_IXGRP = 0o0010;
        /// read by others
        const S_IROTH = 0o0004;
        /// write by others
        const S_IWOTH = 0o0002;
        /// execute/search by others
        const S_IXOTH = 0o0001;
    }
}

impl FilePerm {
    pub fn from_raw_mode(mode: u16) -> Self {
        const PERM_MASK: u16 = 0o7777;
        Self::from_bits_truncate(mode & PERM_MASK)
    }
}

bitflags! {
    pub struct FileFlags: u32 {
        /// Secure deletion.
        const SECURE_DEL = 1 << 0;
        /// Undelete.
        const UNDELETE = 1 << 1;
        /// Compress file.
        const COMPRESS = 1 << 2;
        /// Synchronous updates.
        const SYNC_UPDATE = 1 << 3;
        /// Immutable file.
        const IMMUTABLE = 1 << 4;
        /// Append only.
        const APPEND_ONLY = 1 << 5;
        /// Do not dump file.
        const NO_DUMP = 1 << 6;
        /// Do not update atime.
        const NO_ATIME = 1 << 7;
        /// Dirty.
        const DIRTY = 1 << 8;
        /// One or more compressed clusters.
        const COMPRESS_BLK = 1 << 9;
        /// Do not compress.
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
        /// Dirsync behaviour (directories only).
        const DIR_SYNC = 1 << 16;
        /// Top of directory hierarchies.
        const TOP_DIR = 1 << 17;
        /// Reserved for ext2 lib.
        const RESERVED = 1 << 31;
    }
}
