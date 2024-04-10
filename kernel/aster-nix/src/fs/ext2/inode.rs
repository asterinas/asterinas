// SPDX-License-Identifier: MPL-2.0

use inherit_methods_macro::inherit_methods;

use super::{
    block_ptr::{BidPath, BlockPtrs, Ext2Bid, BID_SIZE, MAX_BLOCK_PTRS},
    blocks_hole::BlocksHoleDesc,
    dir::{DirEntry, DirEntryReader, DirEntryWriter},
    fs::Ext2,
    indirect_block_cache::{IndirectBlock, IndirectBlockCache},
    prelude::*,
};

/// Max length of file name.
pub const MAX_FNAME_LEN: usize = 255;

/// Max path length of the fast symlink.
pub const MAX_FAST_SYMLINK_LEN: usize = MAX_BLOCK_PTRS * BID_SIZE;

/// The Ext2 inode.
pub struct Inode {
    ino: u32,
    block_group_idx: usize,
    inner: RwMutex<Inner>,
    fs: Weak<Ext2>,
}

impl Inode {
    pub(super) fn new(
        ino: u32,
        block_group_idx: usize,
        desc: Dirty<InodeDesc>,
        fs: Weak<Ext2>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            ino,
            block_group_idx,
            inner: RwMutex::new(Inner::new(desc, weak_self.clone(), fs.clone())),
            fs,
        })
    }

    pub fn ino(&self) -> u32 {
        self.ino
    }

    pub(super) fn block_group_idx(&self) -> usize {
        self.block_group_idx
    }

    pub fn fs(&self) -> Arc<Ext2> {
        self.fs.upgrade().unwrap()
    }

    pub fn resize(&self, new_size: usize) -> Result<()> {
        let inner = self.inner.upread();
        if inner.file_type() != FileType::File {
            return_errno!(Errno::EISDIR);
        }
        if new_size == inner.file_size() {
            return Ok(());
        }

        let mut inner = inner.upgrade();
        inner.resize(new_size)?;
        Ok(())
    }

    pub fn page_cache(&self) -> Vmo<Full> {
        self.inner.read().page_cache.pages()
    }

    pub fn create(
        &self,
        name: &str,
        file_type: FileType,
        file_perm: FilePerm,
    ) -> Result<Arc<Self>> {
        let inner = self.inner.upread();
        if inner.file_type() != FileType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if inner.hard_links() == 0 {
            return_errno_with_message!(Errno::ENOENT, "dir removed");
        }
        if name.len() > MAX_FNAME_LEN {
            return_errno!(Errno::ENAMETOOLONG);
        }

        if inner.get_entry(name, 0).is_some() {
            return_errno!(Errno::EEXIST);
        }

        let inode = self
            .fs()
            .create_inode(self.block_group_idx, file_type, file_perm)?;
        let is_dir = file_type == FileType::Dir;
        if let Err(e) = inode.init(self.ino) {
            self.fs().free_inode(inode.ino, is_dir).unwrap();
            return Err(e);
        }
        let new_entry = DirEntry::new(inode.ino, name, file_type);

        let mut inner = inner.upgrade();
        if let Err(e) = inner.append_entry(new_entry, 0) {
            self.fs().free_inode(inode.ino, is_dir).unwrap();
            return Err(e);
        }
        Ok(inode)
    }

    pub fn lookup(&self, name: &str) -> Result<Arc<Self>> {
        let inner = self.inner.read();
        if inner.file_type() != FileType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if inner.hard_links() == 0 {
            return_errno_with_message!(Errno::ENOENT, "dir removed");
        }
        if name.len() > MAX_FNAME_LEN {
            return_errno!(Errno::ENAMETOOLONG);
        }

        let ino = inner
            .get_entry_ino(name, 0)
            .ok_or(Error::new(Errno::ENOENT))?;
        drop(inner);
        self.fs().lookup_inode(ino)
    }

    pub fn link(&self, inode: &Inode, name: &str) -> Result<()> {
        let inner = self.inner.upread();
        if inner.file_type() != FileType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if inner.hard_links() == 0 {
            return_errno_with_message!(Errno::ENOENT, "dir removed");
        }
        if name.len() > MAX_FNAME_LEN {
            return_errno!(Errno::ENAMETOOLONG);
        }
        let inode_type = inode.file_type();
        if inode_type == FileType::Dir {
            return_errno!(Errno::EPERM);
        }

        if inner.get_entry(name, 0).is_some() {
            return_errno!(Errno::EEXIST);
        }

        let new_entry = DirEntry::new(inode.ino, name, inode_type);
        let mut inner = inner.upgrade();
        inner.append_entry(new_entry, 0)?;
        drop(inner);

        inode.inner.write().inc_hard_links();
        Ok(())
    }

    pub fn unlink(&self, name: &str) -> Result<()> {
        let inner = self.inner.upread();
        if inner.file_type() != FileType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if inner.hard_links() == 0 {
            return_errno_with_message!(Errno::ENOENT, "dir removed");
        }
        if name == "." || name == ".." {
            return_errno!(Errno::EISDIR);
        }
        if name.len() > MAX_FNAME_LEN {
            return_errno!(Errno::ENAMETOOLONG);
        }

        let inode = {
            let ino = inner
                .get_entry_ino(name, 0)
                .ok_or(Error::new(Errno::ENOENT))?;
            self.fs().lookup_inode(ino)?
        };
        if inode.file_type() == FileType::Dir {
            return_errno!(Errno::EISDIR);
        }

        let mut inner = inner.upgrade();
        inner.remove_entry(name, 0)?;
        drop(inner);

        inode.inner.write().dec_hard_links();
        Ok(())
    }

    pub fn rmdir(&self, name: &str) -> Result<()> {
        let self_inner = self.inner.upread();
        if self_inner.file_type() != FileType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if self_inner.hard_links() == 0 {
            return_errno_with_message!(Errno::ENOENT, "dir removed");
        }
        if name == "." {
            return_errno_with_message!(Errno::EINVAL, "rmdir on .");
        }
        if name == ".." {
            return_errno_with_message!(Errno::ENOTEMPTY, "rmdir on ..");
        }
        if name.len() > MAX_FNAME_LEN {
            return_errno!(Errno::ENAMETOOLONG);
        }

        let dir_inode = {
            let ino = self_inner
                .get_entry_ino(name, 0)
                .ok_or(Error::new(Errno::ENOENT))?;
            self.fs().lookup_inode(ino)?
        };

        // FIXME: There may be a deadlock here.
        let dir_inner = dir_inode.inner.upread();
        if dir_inner.file_type() != FileType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if dir_inner.entry_count() > 2 {
            return_errno!(Errno::ENOTEMPTY);
        }

        let mut self_inner = self_inner.upgrade();
        self_inner.remove_entry(name, 0)?;
        drop(self_inner);

        let mut dir_inner = dir_inner.upgrade();
        dir_inner.dec_hard_links();
        dir_inner.dec_hard_links(); // For "."
        Ok(())
    }

    /// Rename within its own directory.
    fn rename_within(&self, old_name: &str, new_name: &str) -> Result<()> {
        let self_inner = self.inner.upread();
        if self_inner.file_type() != FileType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if self_inner.hard_links() == 0 {
            return_errno_with_message!(Errno::ENOENT, "dir removed");
        }

        let (src_offset, src_inode) = {
            let (offset, entry) = self_inner
                .get_entry(old_name, 0)
                .ok_or(Error::new(Errno::ENOENT))?;
            (offset, self.fs().lookup_inode(entry.ino())?)
        };

        let Some((dst_offset, dst_entry)) = self_inner.get_entry(new_name, 0) else {
            let mut self_inner = self_inner.upgrade();
            self_inner.rename_entry(old_name, new_name, src_offset)?;
            return Ok(());
        };

        if src_inode.ino == dst_entry.ino() {
            // Same inode, do nothing
            return Ok(());
        }

        let dst_inode = self.fs().lookup_inode(dst_entry.ino())?;
        // FIXME: There may be a deadlock here.
        let dst_inner = dst_inode.inner.upread();
        let dst_inode_type = dst_inner.file_type();
        match (src_inode.file_type(), dst_inode_type) {
            (FileType::Dir, FileType::Dir) => {
                if dst_inner.entry_count() > 2 {
                    return_errno!(Errno::ENOTEMPTY);
                }
            }
            (FileType::Dir, _) => {
                return_errno!(Errno::ENOTDIR);
            }
            (_, FileType::Dir) => {
                return_errno!(Errno::EISDIR);
            }
            _ => {}
        }
        let dst_is_dir = dst_inode_type == FileType::Dir;

        let mut self_inner = self_inner.upgrade();
        self_inner.remove_entry(new_name, dst_offset)?;
        self_inner.rename_entry(old_name, new_name, src_offset)?;
        drop(self_inner);

        let mut dst_inner = dst_inner.upgrade();
        dst_inner.dec_hard_links();
        if dst_is_dir {
            dst_inner.dec_hard_links(); // For "."
        }

        Ok(())
    }

    pub fn rename(&self, old_name: &str, target: &Inode, new_name: &str) -> Result<()> {
        if old_name == "." || old_name == ".." || new_name == "." || new_name == ".." {
            return_errno!(Errno::EISDIR);
        }
        if new_name.len() > MAX_FNAME_LEN || new_name.len() > MAX_FNAME_LEN {
            return_errno!(Errno::ENAMETOOLONG);
        }

        // Rename inside the inode
        if self.ino == target.ino {
            return self.rename_within(old_name, new_name);
        }

        // FIXME: There may be a deadlock here.
        let self_inner = self.inner.upread();
        let target_inner = target.inner.upread();
        if self_inner.file_type() != FileType::Dir || target_inner.file_type() != FileType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if self_inner.hard_links() == 0 || target_inner.hard_links() == 0 {
            return_errno_with_message!(Errno::ENOENT, "dir removed");
        }

        let (src_offset, src_inode) = {
            let (offset, entry) = self_inner
                .get_entry(old_name, 0)
                .ok_or(Error::new(Errno::ENOENT))?;
            (offset, self.fs().lookup_inode(entry.ino())?)
        };
        // Avoid renaming a directory to a subdirectory of itself
        if src_inode.ino == target.ino {
            return_errno!(Errno::EINVAL);
        }
        let src_inode_type = src_inode.file_type();
        let is_dir = src_inode_type == FileType::Dir;

        let Some((dst_offset, dst_entry)) = target_inner.get_entry(new_name, 0) else {
            let mut self_inner = self_inner.upgrade();
            let mut target_inner = target_inner.upgrade();
            self_inner.remove_entry(old_name, src_offset)?;
            let new_entry = DirEntry::new(src_inode.ino, new_name, src_inode_type);
            target_inner.append_entry(new_entry, 0)?;
            drop(self_inner);
            drop(target_inner);

            if is_dir {
                src_inode.inner.write().set_parent_ino(target.ino)?;
            }
            return Ok(());
        };

        if src_inode.ino == dst_entry.ino() {
            // Same inode, do nothing
            return Ok(());
        }

        // Avoid renaming a subdirectory to a directory.
        if self.ino == dst_entry.ino() {
            return_errno!(Errno::ENOTEMPTY);
        }

        let dst_inode = self.fs().lookup_inode(dst_entry.ino())?;
        // FIXME: There may be a deadlock here.
        let dst_inner = dst_inode.inner.upread();
        let dst_inode_type = dst_inner.file_type();
        match (src_inode_type, dst_inode_type) {
            (FileType::Dir, FileType::Dir) => {
                if dst_inner.entry_count() > 2 {
                    return_errno!(Errno::ENOTEMPTY);
                }
            }
            (FileType::Dir, _) => {
                return_errno!(Errno::ENOTDIR);
            }
            (_, FileType::Dir) => {
                return_errno!(Errno::EISDIR);
            }
            _ => {}
        }
        let mut self_inner = self_inner.upgrade();
        let mut target_inner = target_inner.upgrade();
        self_inner.remove_entry(old_name, src_offset)?;
        target_inner.remove_entry(new_name, dst_offset)?;
        let new_entry = DirEntry::new(src_inode.ino, new_name, src_inode_type);
        target_inner.append_entry(new_entry, 0)?;
        drop(self_inner);
        drop(target_inner);

        let mut dst_inner = dst_inner.upgrade();
        dst_inner.dec_hard_links();
        if is_dir {
            dst_inner.dec_hard_links(); // For "."
        }
        drop(dst_inner);

        if is_dir {
            src_inode.inner.write().set_parent_ino(target.ino)?;
        }

        Ok(())
    }

    pub fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        let inner = self.inner.read();
        if inner.file_type() != FileType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if inner.hard_links() == 0 {
            return_errno_with_message!(Errno::ENOENT, "dir removed");
        }

        let try_readdir = |offset: &mut usize, visitor: &mut dyn DirentVisitor| -> Result<()> {
            let dir_entry_reader = DirEntryReader::new(&inner.page_cache, *offset);
            for (_, dir_entry) in dir_entry_reader {
                visitor.visit(
                    dir_entry.name(),
                    dir_entry.ino() as u64,
                    InodeType::from(dir_entry.type_()),
                    dir_entry.record_len(),
                )?;
                *offset += dir_entry.record_len();
            }

            Ok(())
        };

        let mut iterate_offset = offset;
        match try_readdir(&mut iterate_offset, visitor) {
            Err(e) if iterate_offset == offset => Err(e),
            _ => Ok(iterate_offset - offset),
        }
    }

    pub fn write_link(&self, target: &str) -> Result<()> {
        let mut inner = self.inner.write();
        if inner.file_type() != FileType::Symlink {
            return_errno!(Errno::EISDIR);
        }

        inner.write_link(target)?;
        Ok(())
    }

    pub fn read_link(&self) -> Result<String> {
        let inner = self.inner.read();
        if inner.file_type() != FileType::Symlink {
            return_errno!(Errno::EISDIR);
        }

        inner.read_link()
    }

    pub fn set_device_id(&self, device_id: u64) -> Result<()> {
        let mut inner = self.inner.write();
        let file_type = inner.file_type();
        if file_type != FileType::Block && file_type != FileType::Char {
            return_errno!(Errno::EISDIR);
        }

        inner.set_device_id(device_id);
        Ok(())
    }

    pub fn device_id(&self) -> u64 {
        let inner = self.inner.read();
        let file_type = inner.file_type();
        if file_type != FileType::Block && file_type != FileType::Char {
            return 0;
        }
        inner.device_id()
    }

    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        let inner = self.inner.read();
        if inner.file_type() != FileType::File {
            return_errno!(Errno::EISDIR);
        }

        inner.read_at(offset, buf)
    }

    // The offset and the length of buffer must be multiples of the block size.
    pub fn read_direct_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        let inner = self.inner.read();
        if inner.file_type() != FileType::File {
            return_errno!(Errno::EISDIR);
        }
        if !is_block_aligned(offset) || !is_block_aligned(buf.len()) {
            return_errno_with_message!(Errno::EINVAL, "not block-aligned");
        }

        inner.read_direct_at(offset, buf)
    }

    pub fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        let inner = self.inner.upread();
        if inner.file_type() != FileType::File {
            return_errno!(Errno::EISDIR);
        }

        let file_size = inner.file_size();
        let new_size = offset + buf.len();
        if new_size > file_size {
            let mut inner = inner.upgrade();
            inner.extend_write_at(offset, buf)?;
        } else {
            inner.write_at(offset, buf)?;
        }

        Ok(buf.len())
    }

    // The offset and the length of buffer must be multiples of the block size.
    pub fn write_direct_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        let inner = self.inner.upread();
        if inner.file_type() != FileType::File {
            return_errno!(Errno::EISDIR);
        }
        if !is_block_aligned(offset) || !is_block_aligned(buf.len()) {
            return_errno_with_message!(Errno::EINVAL, "not block aligned");
        }

        let mut inner = inner.upgrade();
        inner.write_direct_at(offset, buf)?;
        Ok(buf.len())
    }

    fn init(&self, dir_ino: u32) -> Result<()> {
        let mut inner = self.inner.write();
        match inner.file_type() {
            FileType::Dir => {
                inner.init_dir(self.ino, dir_ino)?;
            }
            _ => {
                // TODO: Reserve serval blocks for regular file ?
            }
        }
        Ok(())
    }

    pub fn sync_all(&self) -> Result<()> {
        let inner = self.inner.read();
        inner.sync_data()?;
        inner.sync_metadata()?;
        Ok(())
    }
}

#[inherit_methods(from = "self.inner.read()")]
impl Inode {
    pub fn file_size(&self) -> usize;
    pub fn file_type(&self) -> FileType;
    pub fn file_perm(&self) -> FilePerm;
    pub fn uid(&self) -> u32;
    pub fn gid(&self) -> u32;
    pub fn file_flags(&self) -> FileFlags;
    pub fn hard_links(&self) -> u16;
    pub fn blocks_count(&self) -> Ext2Bid;
    pub fn acl(&self) -> Option<Bid>;
    pub fn atime(&self) -> Duration;
    pub fn mtime(&self) -> Duration;
    pub fn ctime(&self) -> Duration;
    pub fn sync_data(&self) -> Result<()>;
    pub fn sync_metadata(&self) -> Result<()>;
}

#[inherit_methods(from = "self.inner.write()")]
impl Inode {
    pub fn set_file_perm(&self, perm: FilePerm);
    pub fn set_uid(&self, uid: u32);
    pub fn set_gid(&self, gid: u32);
    pub fn set_atime(&self, time: Duration);
    pub fn set_mtime(&self, time: Duration);
}

impl Debug for Inode {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("Inode")
            .field("ino", &self.ino)
            .field("block_group_idx", &self.block_group_idx)
            .finish()
    }
}

struct Inner {
    inode_impl: Arc<InodeImpl>,
    page_cache: PageCache,
}

#[inherit_methods(from = "self.inode_impl")]
impl Inner {
    pub fn file_size(&self) -> usize;
    pub fn file_type(&self) -> FileType;
    pub fn file_perm(&self) -> FilePerm;
    pub fn set_file_perm(&mut self, perm: FilePerm);
    pub fn uid(&self) -> u32;
    pub fn set_uid(&mut self, uid: u32);
    pub fn gid(&self) -> u32;
    pub fn set_gid(&mut self, gid: u32);
    pub fn file_flags(&self) -> FileFlags;
    pub fn hard_links(&self) -> u16;
    pub fn inc_hard_links(&mut self);
    pub fn dec_hard_links(&mut self);
    pub fn blocks_count(&self) -> Ext2Bid;
    pub fn acl(&self) -> Option<Bid>;
    pub fn atime(&self) -> Duration;
    pub fn set_atime(&mut self, time: Duration);
    pub fn mtime(&self) -> Duration;
    pub fn set_mtime(&mut self, time: Duration);
    pub fn ctime(&self) -> Duration;
    pub fn set_device_id(&mut self, device_id: u64);
    pub fn device_id(&self) -> u64;
    pub fn sync_metadata(&self) -> Result<()>;
}

impl Inner {
    pub fn new(desc: Dirty<InodeDesc>, weak_self: Weak<Inode>, fs: Weak<Ext2>) -> Self {
        let num_page_bytes = desc.num_page_bytes();
        let inode_impl = InodeImpl::new(desc, weak_self, fs);
        Self {
            page_cache: PageCache::with_capacity(num_page_bytes, Arc::downgrade(&inode_impl) as _)
                .unwrap(),
            inode_impl,
        }
    }

    pub fn resize(&mut self, new_size: usize) -> Result<()> {
        self.inode_impl.resize(new_size)?;
        self.page_cache.pages().resize(new_size)?;
        Ok(())
    }

    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        let (offset, read_len) = {
            let file_size = self.inode_impl.file_size();
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
        let (offset, read_len) = {
            let file_size = self.inode_impl.file_size();
            let start = file_size.min(offset).align_down(BLOCK_SIZE);
            let end = file_size.min(offset + buf.len()).align_down(BLOCK_SIZE);
            (start, end - start)
        };
        self.page_cache
            .pages()
            .decommit(offset..offset + read_len)?;

        let mut buf_offset = 0;
        for bid in Bid::from_offset(offset)..Bid::from_offset(offset + read_len) {
            let frame = VmAllocOptions::new(1).uninit(true).alloc_single().unwrap();
            self.inode_impl
                .read_block_sync(bid.to_raw() as Ext2Bid, &frame)?;
            frame.read_bytes(0, &mut buf[buf_offset..buf_offset + BLOCK_SIZE])?;
            buf_offset += BLOCK_SIZE;
        }
        Ok(read_len)
    }

    pub fn write_at(&self, offset: usize, buf: &[u8]) -> Result<()> {
        self.page_cache.pages().write_bytes(offset, buf)?;
        Ok(())
    }

    pub fn extend_write_at(&mut self, offset: usize, buf: &[u8]) -> Result<()> {
        let new_size = offset + buf.len();
        self.page_cache.pages().resize(new_size)?;
        self.page_cache.pages().write_bytes(offset, buf)?;
        self.inode_impl.resize(new_size)?;
        Ok(())
    }

    pub fn write_direct_at(&mut self, offset: usize, buf: &[u8]) -> Result<()> {
        let file_size = self.inode_impl.file_size();
        let end_offset = offset + buf.len();

        let start = offset.min(file_size);
        let end = end_offset.min(file_size);
        self.page_cache.pages().decommit(start..end)?;

        if end_offset > file_size {
            self.page_cache.pages().resize(end_offset)?;
            self.inode_impl.resize(end_offset)?;
        }

        let mut buf_offset = 0;
        for bid in Bid::from_offset(offset)..Bid::from_offset(end_offset) {
            let frame = {
                let frame = VmAllocOptions::new(1).uninit(true).alloc_single().unwrap();
                frame.write_bytes(0, &buf[buf_offset..buf_offset + BLOCK_SIZE])?;
                frame
            };
            self.inode_impl
                .write_block_sync(bid.to_raw() as Ext2Bid, &frame)?;
            buf_offset += BLOCK_SIZE;
        }

        Ok(())
    }

    pub fn write_link(&mut self, target: &str) -> Result<()> {
        if target.len() <= MAX_FAST_SYMLINK_LEN {
            return self.inode_impl.write_link(target);
        }

        self.page_cache.pages().resize(target.len())?;
        self.page_cache.pages().write_bytes(0, target.as_bytes())?;
        let file_size = self.inode_impl.file_size();
        if file_size != target.len() {
            self.inode_impl.resize(target.len())?;
        }
        Ok(())
    }

    pub fn read_link(&self) -> Result<String> {
        let file_size = self.inode_impl.file_size();
        if file_size <= MAX_FAST_SYMLINK_LEN {
            return self.inode_impl.read_link();
        }

        let mut symlink = vec![0u8; file_size];
        self.page_cache
            .pages()
            .read_bytes(0, symlink.as_mut_slice())?;

        Ok(String::from_utf8(symlink)?)
    }

    fn init_dir(&mut self, self_ino: u32, parent_ino: u32) -> Result<()> {
        self.append_entry(DirEntry::self_entry(self_ino), 0)?;
        self.append_entry(DirEntry::parent_entry(parent_ino), 0)?;
        Ok(())
    }

    pub fn get_entry_ino(&self, name: &str, offset: usize) -> Option<u32> {
        self.get_entry(name, offset).map(|(_, entry)| entry.ino())
    }

    pub fn get_entry(&self, name: &str, offset: usize) -> Option<(usize, DirEntry)> {
        DirEntryReader::new(&self.page_cache, offset).find(|(offset, entry)| entry.name() == name)
    }

    pub fn entry_count(&self) -> usize {
        DirEntryReader::new(&self.page_cache, 0).count()
    }

    pub fn append_entry(&mut self, entry: DirEntry, offset: usize) -> Result<()> {
        let is_dir = entry.type_() == FileType::Dir;
        let is_parent = entry.name() == "..";

        DirEntryWriter::new(&self.page_cache, offset).append_entry(entry)?;
        let file_size = self.inode_impl.file_size();
        let page_cache_size = self.page_cache.pages().size();
        if page_cache_size > file_size {
            self.inode_impl.resize(page_cache_size)?;
        }
        if is_dir && !is_parent {
            self.inc_hard_links(); // for ".."
        }
        Ok(())
    }

    pub fn remove_entry(&mut self, name: &str, offset: usize) -> Result<()> {
        let entry = DirEntryWriter::new(&self.page_cache, offset).remove_entry(name)?;
        let is_dir = entry.type_() == FileType::Dir;
        let file_size = self.inode_impl.file_size();
        let page_cache_size = self.page_cache.pages().size();
        if page_cache_size < file_size {
            self.inode_impl.resize(page_cache_size)?;
        }
        if is_dir {
            self.dec_hard_links(); // for ".."
        }
        Ok(())
    }

    pub fn rename_entry(&mut self, old_name: &str, new_name: &str, offset: usize) -> Result<()> {
        DirEntryWriter::new(&self.page_cache, offset).rename_entry(old_name, new_name)?;
        let file_size = self.inode_impl.file_size();
        let page_cache_size = self.page_cache.pages().size();
        if page_cache_size != file_size {
            self.inode_impl.resize(page_cache_size)?;
        }
        Ok(())
    }

    pub fn set_parent_ino(&mut self, parent_ino: u32) -> Result<()> {
        let (offset, mut entry) = self.get_entry("..", 0).unwrap();
        entry.set_ino(parent_ino);
        DirEntryWriter::new(&self.page_cache, offset).write_entry(&entry)?;
        Ok(())
    }

    pub fn sync_data(&self) -> Result<()> {
        // Writes back the data in page cache.
        let file_size = self.inode_impl.file_size();
        self.page_cache.evict_range(0..file_size)?;

        // Writes back the data holes
        self.inode_impl.sync_data_holes()?;
        Ok(())
    }
}

struct InodeImpl(RwMutex<InodeImpl_>);

struct InodeImpl_ {
    desc: Dirty<InodeDesc>,
    blocks_hole_desc: RwLock<BlocksHoleDesc>,
    indirect_blocks: RwMutex<IndirectBlockCache>,
    is_freed: bool,
    last_alloc_device_bid: Option<Ext2Bid>,
    weak_self: Weak<Inode>,
}

impl InodeImpl_ {
    pub fn new(desc: Dirty<InodeDesc>, weak_self: Weak<Inode>, fs: Weak<Ext2>) -> Self {
        Self {
            blocks_hole_desc: RwLock::new(BlocksHoleDesc::new(desc.blocks_count() as usize)),
            desc,
            indirect_blocks: RwMutex::new(IndirectBlockCache::new(fs)),
            is_freed: false,
            last_alloc_device_bid: None,
            weak_self,
        }
    }

    pub fn inode(&self) -> Arc<Inode> {
        self.weak_self.upgrade().unwrap()
    }

    pub fn fs(&self) -> Arc<Ext2> {
        self.inode().fs()
    }

    pub fn read_block_async(&self, bid: Ext2Bid, block: &VmFrame) -> Result<BioWaiter> {
        if bid >= self.desc.blocks_count() {
            return_errno!(Errno::EINVAL);
        }

        if self.blocks_hole_desc.read().is_hole(bid as usize) {
            block.writer().fill(0);
            return Ok(BioWaiter::new());
        }

        let device_range = DeviceRangeReader::new(self, bid..bid + 1)?.read()?;
        self.fs().read_block_async(device_range.start, block)
    }

    pub fn read_block_sync(&self, bid: Ext2Bid, block: &VmFrame) -> Result<()> {
        match self.read_block_async(bid, block)?.wait() {
            Some(BioStatus::Complete) => Ok(()),
            _ => return_errno!(Errno::EIO),
        }
    }

    pub fn write_block_async(&self, bid: Ext2Bid, block: &VmFrame) -> Result<BioWaiter> {
        if bid >= self.desc.blocks_count() {
            return_errno!(Errno::EINVAL);
        }

        let device_range = DeviceRangeReader::new(self, bid..bid + 1)?.read()?;
        let waiter = self.fs().write_block_async(device_range.start, block)?;

        // FIXME: Unset the block hole in the callback function of bio.
        self.blocks_hole_desc.write().unset(bid as usize);
        Ok(waiter)
    }

    pub fn write_block_sync(&self, bid: Ext2Bid, block: &VmFrame) -> Result<()> {
        match self.write_block_async(bid, block)?.wait() {
            Some(BioStatus::Complete) => Ok(()),
            _ => return_errno!(Errno::EIO),
        }
    }

    pub fn resize(&mut self, new_size: usize) -> Result<()> {
        let old_size = self.desc.size;
        if new_size > old_size {
            self.expand(new_size)?;
        } else {
            self.shrink(new_size);
        }
        Ok(())
    }

    /// Expands inode size.
    ///
    /// After a successful expansion, the size will be enlarged to `new_size`,
    /// which may result in an increased block count.
    fn expand(&mut self, new_size: usize) -> Result<()> {
        let new_blocks = self.desc.size_to_blocks(new_size);
        let old_blocks = self.desc.blocks_count();

        // Expands block count if necessary
        if new_blocks > old_blocks {
            if new_blocks - old_blocks > self.fs().super_block().free_blocks_count() {
                return_errno_with_message!(Errno::ENOSPC, "not enough free blocks");
            }
            self.expand_blocks(old_blocks..new_blocks)?;
            self.blocks_hole_desc.write().resize(new_blocks as usize);
        }

        // Expands the size
        self.desc.size = new_size;
        Ok(())
    }

    /// Expands inode blocks.
    ///
    /// After a successful expansion, the block count will be enlarged to `range.end`.
    fn expand_blocks(&mut self, range: Range<Ext2Bid>) -> Result<()> {
        let mut current_range = range.clone();
        while !current_range.is_empty() {
            let Ok(expand_cnt) = self.try_expand_blocks(current_range.clone()) else {
                self.shrink_blocks(range.start..current_range.start);
                return_errno_with_message!(Errno::ENOSPC, "can not allocate blocks");
            };
            current_range.start += expand_cnt;
        }

        Ok(())
    }

    /// Attempts to expand a range of blocks and returns the number of consecutive
    /// blocks successfully allocated.
    ///
    /// Note that the returned number may be less than the requested range if there
    /// isn't enough consecutive space available or if there is a necessity to allocate
    /// indirect blocks.
    fn try_expand_blocks(&mut self, range: Range<Ext2Bid>) -> Result<Ext2Bid> {
        // Calculates the maximum number of consecutive blocks that can be allocated in
        // this round, as well as the number of additional indirect blocks required for
        // the allocation.
        let (max_cnt, indirect_cnt) = {
            let bid_path = BidPath::from(range.start);
            let max_cnt = (range.len() as Ext2Bid).min(bid_path.cnt_to_next_indirect());
            let indirect_cnt = match bid_path {
                BidPath::Direct(_) => 0,
                BidPath::Indirect(0) => 1,
                BidPath::Indirect(_) => 0,
                BidPath::DbIndirect(0, 0) => 2,
                BidPath::DbIndirect(_, 0) => 1,
                BidPath::DbIndirect(_, _) => 0,
                BidPath::TbIndirect(0, 0, 0) => 3,
                BidPath::TbIndirect(_, 0, 0) => 2,
                BidPath::TbIndirect(_, _, 0) => 1,
                BidPath::TbIndirect(_, _, _) => 0,
            };
            (max_cnt, indirect_cnt)
        };

        // Calculates the block_group_idx to advise the filesystem on which group
        // to prioritize for allocation.
        let block_group_idx = self
            .last_alloc_device_bid
            .map_or(self.inode().block_group_idx, |id| {
                ((id + 1) / self.fs().blocks_per_group()) as usize
            });

        // Allocates the blocks only, no indirect blocks are required.
        if indirect_cnt == 0 {
            let device_range = self
                .fs()
                .alloc_blocks(block_group_idx, max_cnt)
                .ok_or_else(|| Error::new(Errno::ENOSPC))?;
            if let Err(e) = self.set_device_range(range.start, device_range.clone()) {
                self.fs().free_blocks(device_range).unwrap();
                return Err(e);
            }
            self.desc.blocks_count = range.start + device_range.len() as Ext2Bid;
            self.last_alloc_device_bid = Some(device_range.end - 1);
            return Ok(device_range.len() as Ext2Bid);
        }

        // Allocates the required additional indirect blocks and at least one block.
        let (indirect_bids, device_range) = {
            let mut indirect_bids: Vec<Ext2Bid> = Vec::with_capacity(indirect_cnt as usize);
            let mut total_cnt = max_cnt + indirect_cnt;
            let mut device_range: Option<Range<Ext2Bid>> = None;
            while device_range.is_none() {
                let Some(mut range) = self.fs().alloc_blocks(block_group_idx, total_cnt) else {
                    for indirect_bid in indirect_bids.iter() {
                        self.fs()
                            .free_blocks(*indirect_bid..*indirect_bid + 1)
                            .unwrap();
                    }
                    return_errno!(Errno::ENOSPC);
                };
                total_cnt -= range.len() as Ext2Bid;

                // Stores the bids for indirect blocks.
                while (indirect_bids.len() as Ext2Bid) < indirect_cnt && !range.is_empty() {
                    indirect_bids.push(range.start);
                    range.start += 1;
                }

                if !range.is_empty() {
                    device_range = Some(range);
                }
            }

            (indirect_bids, device_range.unwrap())
        };

        if let Err(e) = self.set_indirect_bids(range.start, &indirect_bids) {
            self.free_indirect_blocks_required_by(range.start).unwrap();
            return Err(e);
        }

        if let Err(e) = self.set_device_range(range.start, device_range.clone()) {
            self.fs().free_blocks(device_range).unwrap();
            self.free_indirect_blocks_required_by(range.start).unwrap();
            return Err(e);
        }

        self.desc.blocks_count = range.start + device_range.len() as Ext2Bid;
        self.last_alloc_device_bid = Some(device_range.end - 1);
        Ok(device_range.len() as Ext2Bid)
    }

    /// Sets the device block IDs for a specified range.
    ///
    /// It updates the mapping between the file's block IDs and the device's block IDs
    /// starting from `start_bid`. It maps each block ID in the file to the corresponding
    /// block ID on the device based on the provided `device_range`.
    fn set_device_range(&mut self, start_bid: Ext2Bid, device_range: Range<Ext2Bid>) -> Result<()> {
        match BidPath::from(start_bid) {
            BidPath::Direct(idx) => {
                for (i, bid) in device_range.enumerate() {
                    self.desc.block_ptrs.set_direct(idx as usize + i, bid);
                }
            }
            BidPath::Indirect(idx) => {
                let indirect_bid = self.desc.block_ptrs.indirect();
                assert!(indirect_bid != 0);
                let mut indirect_blocks = self.indirect_blocks.write();
                let indirect_block = indirect_blocks.find_mut(indirect_bid)?;
                for (i, bid) in device_range.enumerate() {
                    indirect_block.write_bid(idx as usize + i, &bid)?;
                }
            }
            BidPath::DbIndirect(lvl1_idx, lvl2_idx) => {
                let mut indirect_blocks = self.indirect_blocks.write();
                let lvl1_indirect_bid = {
                    let db_indirect_bid = self.desc.block_ptrs.db_indirect();
                    assert!(db_indirect_bid != 0);
                    let db_indirect_block = indirect_blocks.find(db_indirect_bid)?;
                    db_indirect_block.read_bid(lvl1_idx as usize)?
                };
                assert!(lvl1_indirect_bid != 0);

                let lvl1_indirect_block = indirect_blocks.find_mut(lvl1_indirect_bid)?;
                for (i, bid) in device_range.enumerate() {
                    lvl1_indirect_block.write_bid(lvl2_idx as usize + i, &bid)?;
                }
            }
            BidPath::TbIndirect(lvl1_idx, lvl2_idx, lvl3_idx) => {
                let mut indirect_blocks = self.indirect_blocks.write();
                let lvl2_indirect_bid = {
                    let lvl1_indirect_bid = {
                        let tb_indirect_bid = self.desc.block_ptrs.tb_indirect();
                        assert!(tb_indirect_bid != 0);
                        let tb_indirect_block = indirect_blocks.find(tb_indirect_bid)?;
                        tb_indirect_block.read_bid(lvl1_idx as usize)?
                    };
                    assert!(lvl1_indirect_bid != 0);
                    let lvl1_indirect_block = indirect_blocks.find(lvl1_indirect_bid)?;
                    lvl1_indirect_block.read_bid(lvl2_idx as usize)?
                };
                assert!(lvl2_indirect_bid != 0);

                let lvl2_indirect_block = indirect_blocks.find_mut(lvl2_indirect_bid)?;
                for (i, bid) in device_range.enumerate() {
                    lvl2_indirect_block.write_bid(lvl3_idx as usize + i, &bid)?;
                }
            }
        }
        Ok(())
    }

    /// Sets the device block IDs for indirect blocks required by a specific block ID.
    ///
    /// It assigns a sequence of block IDs (`indirect_bids`) on the device to be used
    /// as indirect blocks for a given file block ID (`bid`).
    fn set_indirect_bids(&mut self, bid: Ext2Bid, indirect_bids: &[Ext2Bid]) -> Result<()> {
        assert!((1..=3).contains(&indirect_bids.len()));

        let mut indirect_blocks = self.indirect_blocks.write();
        let bid_path = BidPath::from(bid);
        for indirect_bid in indirect_bids.iter() {
            let indirect_block = IndirectBlock::alloc()?;
            indirect_blocks.insert(*indirect_bid, indirect_block)?;

            match bid_path {
                BidPath::Indirect(idx) => {
                    assert_eq!(idx, 0);
                    self.desc.block_ptrs.set_indirect(*indirect_bid);
                }
                BidPath::DbIndirect(lvl1_idx, lvl2_idx) => {
                    assert_eq!(lvl2_idx, 0);
                    if self.desc.block_ptrs.db_indirect() == 0 {
                        self.desc.block_ptrs.set_db_indirect(*indirect_bid);
                    } else {
                        let db_indirect_block =
                            indirect_blocks.find_mut(self.desc.block_ptrs.db_indirect())?;
                        db_indirect_block.write_bid(lvl1_idx as usize, indirect_bid)?;
                    }
                }
                BidPath::TbIndirect(lvl1_idx, lvl2_idx, lvl3_idx) => {
                    assert_eq!(lvl3_idx, 0);
                    if self.desc.block_ptrs.tb_indirect() == 0 {
                        self.desc.block_ptrs.set_tb_indirect(*indirect_bid);
                    } else {
                        let lvl1_indirect_bid = {
                            let tb_indirect_block =
                                indirect_blocks.find(self.desc.block_ptrs.tb_indirect())?;
                            tb_indirect_block.read_bid(lvl1_idx as usize)?
                        };

                        if lvl1_indirect_bid == 0 {
                            let tb_indirect_block =
                                indirect_blocks.find_mut(self.desc.block_ptrs.tb_indirect())?;
                            tb_indirect_block.write_bid(lvl1_idx as usize, indirect_bid)?;
                        } else {
                            let lvl1_indirect_block =
                                indirect_blocks.find_mut(lvl1_indirect_bid)?;
                            lvl1_indirect_block.write_bid(lvl2_idx as usize, indirect_bid)?;
                        }
                    }
                }
                BidPath::Direct(_) => panic!(),
            }
        }

        Ok(())
    }

    /// Shrinks inode size.
    ///
    /// After the reduction, the size will be shrinked to `new_size`,
    /// which may result in an decreased block count.
    fn shrink(&mut self, new_size: usize) {
        let new_blocks = self.desc.size_to_blocks(new_size);
        let old_blocks = self.desc.blocks_count();

        // Shrinks block count if necessary
        if new_blocks < old_blocks {
            self.shrink_blocks(new_blocks..old_blocks);
            self.blocks_hole_desc.write().resize(new_blocks as usize);
        }

        // Shrinks the size
        self.desc.size = new_size;
    }

    /// Shrinks inode blocks.
    ///
    /// After the reduction, the block count will be decreased to `range.start`.
    fn shrink_blocks(&mut self, range: Range<Ext2Bid>) {
        let mut current_range = range.clone();
        while !current_range.is_empty() {
            let free_cnt = self.try_shrink_blocks(current_range.clone());
            current_range.end -= free_cnt;
        }

        self.desc.blocks_count = range.start;
        self.last_alloc_device_bid = if range.start == 0 {
            None
        } else {
            Some(
                DeviceRangeReader::new(self, (range.start - 1)..range.start)
                    .unwrap()
                    .read()
                    .unwrap()
                    .start,
            )
        };
    }

    /// Attempts to shrink a range of blocks and returns the number of blocks
    /// successfully freed.
    ///
    /// Note that the returned number may be less than the requested range if needs
    /// to free the indirect blocks that are no longer required.
    fn try_shrink_blocks(&mut self, range: Range<Ext2Bid>) -> Ext2Bid {
        // Calculates the maximum range of blocks that can be freed in this round.
        let range = {
            let max_cnt = (range.len() as Ext2Bid)
                .min(BidPath::from(range.end - 1).last_lvl_idx() as Ext2Bid + 1);
            (range.end - max_cnt)..range.end
        };

        let fs = self.fs();
        let device_range_reader = DeviceRangeReader::new(self, range.clone()).unwrap();
        for device_range in device_range_reader {
            fs.free_blocks(device_range.clone()).unwrap();
        }

        self.free_indirect_blocks_required_by(range.start).unwrap();
        range.len() as Ext2Bid
    }

    /// Frees the indirect blocks required by the specified block ID.
    ///
    /// It ensures that the indirect blocks that are required by the block ID
    /// are properly released.
    fn free_indirect_blocks_required_by(&mut self, bid: Ext2Bid) -> Result<()> {
        let bid_path = BidPath::from(bid);
        if bid_path.last_lvl_idx() != 0 {
            return Ok(());
        }
        if bid == 0 {
            return Ok(());
        }

        match bid_path {
            BidPath::Indirect(_) => {
                let indirect_bid = self.desc.block_ptrs.indirect();
                if indirect_bid == 0 {
                    return Ok(());
                }

                self.desc.block_ptrs.set_indirect(0);
                self.indirect_blocks.write().remove(indirect_bid);
                self.fs()
                    .free_blocks(indirect_bid..indirect_bid + 1)
                    .unwrap();
            }
            BidPath::DbIndirect(lvl1_idx, _) => {
                let db_indirect_bid = self.desc.block_ptrs.db_indirect();
                if db_indirect_bid == 0 {
                    return Ok(());
                }

                let mut indirect_blocks = self.indirect_blocks.write();
                let lvl1_indirect_bid = {
                    let db_indirect_block = indirect_blocks.find(db_indirect_bid)?;
                    db_indirect_block.read_bid(lvl1_idx as usize)?
                };
                if lvl1_indirect_bid != 0 {
                    indirect_blocks.remove(lvl1_indirect_bid);
                    self.fs()
                        .free_blocks(lvl1_indirect_bid..lvl1_indirect_bid + 1)
                        .unwrap();
                }
                if lvl1_idx == 0 {
                    self.desc.block_ptrs.set_db_indirect(0);
                    indirect_blocks.remove(db_indirect_bid);
                    self.fs()
                        .free_blocks(db_indirect_bid..db_indirect_bid + 1)
                        .unwrap();
                }
            }
            BidPath::TbIndirect(lvl1_idx, lvl2_idx, _) => {
                let tb_indirect_bid = self.desc.block_ptrs.tb_indirect();
                if tb_indirect_bid == 0 {
                    return Ok(());
                }

                let mut indirect_blocks = self.indirect_blocks.write();
                let lvl1_indirect_bid = {
                    let tb_indirect_block = indirect_blocks.find(tb_indirect_bid)?;
                    tb_indirect_block.read_bid(lvl1_idx as usize)?
                };
                if lvl1_indirect_bid != 0 {
                    let lvl2_indirect_bid = {
                        let lvl1_indirect_block = indirect_blocks.find(lvl1_indirect_bid)?;
                        lvl1_indirect_block.read_bid(lvl2_idx as usize)?
                    };
                    if lvl2_indirect_bid != 0 {
                        indirect_blocks.remove(lvl2_indirect_bid);
                        self.fs()
                            .free_blocks(lvl2_indirect_bid..lvl2_indirect_bid + 1)
                            .unwrap();
                    }
                    if lvl2_idx == 0 {
                        indirect_blocks.remove(lvl1_indirect_bid);
                        self.fs()
                            .free_blocks(lvl1_indirect_bid..lvl1_indirect_bid + 1)
                            .unwrap();
                    }
                }

                if lvl2_idx == 0 && lvl1_idx == 0 {
                    self.desc.block_ptrs.set_tb_indirect(0);
                    indirect_blocks.remove(tb_indirect_bid);
                    self.fs()
                        .free_blocks(tb_indirect_bid..tb_indirect_bid + 1)
                        .unwrap();
                }
            }
            BidPath::Direct(_) => panic!(),
        }

        Ok(())
    }
}

/// A reader to get the corresponding device block IDs for a specified range.
///
/// It calculates and returns the range of block IDs on the device that would map to
/// the file's block range. This is useful for translating file-level block addresses
/// to their locations on the physical storage device.
struct DeviceRangeReader<'a> {
    inode: &'a InodeImpl_,
    indirect_blocks: RwMutexWriteGuard<'a, IndirectBlockCache>,
    range: Range<Ext2Bid>,
    indirect_block: Option<IndirectBlock>,
}

impl<'a> DeviceRangeReader<'a> {
    /// Creates a new reader.
    ///
    /// # Panic
    ///
    /// If the 'range' is empty, this method will panic.
    pub fn new(inode: &'a InodeImpl_, range: Range<Ext2Bid>) -> Result<Self> {
        assert!(!range.is_empty());

        let mut reader = Self {
            indirect_blocks: inode.indirect_blocks.write(),
            inode,
            range,
            indirect_block: None,
        };
        reader.update_indirect_block()?;
        Ok(reader)
    }

    /// Reads the corresponding device block IDs for a specified range.
    ///
    /// Note that the returned device range size may be smaller than the requested range
    /// due to possible inconsecutive block allocation.
    pub fn read(&mut self) -> Result<Range<Ext2Bid>> {
        let bid_path = BidPath::from(self.range.start);
        let max_cnt = self
            .range
            .len()
            .min(bid_path.cnt_to_next_indirect() as usize);
        let start_idx = bid_path.last_lvl_idx();

        // Reads the device block ID range
        let mut device_range: Option<Range<Ext2Bid>> = None;
        for i in start_idx..start_idx + max_cnt {
            let device_bid = match &self.indirect_block {
                None => self.inode.desc.block_ptrs.direct(i),
                Some(indirect_block) => indirect_block.read_bid(i)?,
            };
            match device_range {
                Some(ref mut range) => {
                    if device_bid == range.end {
                        range.end += 1;
                    } else {
                        break;
                    }
                }
                None => {
                    device_range = Some(device_bid..device_bid + 1);
                }
            }
        }
        let device_range = device_range.unwrap();

        // Updates the range
        self.range.start += device_range.len() as Ext2Bid;
        if device_range.len() == max_cnt {
            // Updates the indirect block
            self.update_indirect_block()?;
        }

        Ok(device_range)
    }

    fn update_indirect_block(&mut self) -> Result<()> {
        let bid_path = BidPath::from(self.range.start);
        match bid_path {
            BidPath::Direct(_) => {
                self.indirect_block = None;
            }
            BidPath::Indirect(_) => {
                let indirect_bid = self.inode.desc.block_ptrs.indirect();
                let indirect_block = self.indirect_blocks.find(indirect_bid)?;
                self.indirect_block = Some(indirect_block.clone());
            }
            BidPath::DbIndirect(lvl1_idx, _) => {
                let lvl1_indirect_bid = {
                    let db_indirect_block = self
                        .indirect_blocks
                        .find(self.inode.desc.block_ptrs.db_indirect())?;
                    db_indirect_block.read_bid(lvl1_idx as usize)?
                };
                let lvl1_indirect_block = self.indirect_blocks.find(lvl1_indirect_bid)?;
                self.indirect_block = Some(lvl1_indirect_block.clone())
            }
            BidPath::TbIndirect(lvl1_idx, lvl2_idx, _) => {
                let lvl2_indirect_bid = {
                    let lvl1_indirect_bid = {
                        let tb_indirect_block = self
                            .indirect_blocks
                            .find(self.inode.desc.block_ptrs.tb_indirect())?;
                        tb_indirect_block.read_bid(lvl1_idx as usize)?
                    };
                    let lvl1_indirect_block = self.indirect_blocks.find(lvl1_indirect_bid)?;
                    lvl1_indirect_block.read_bid(lvl2_idx as usize)?
                };
                let lvl2_indirect_block = self.indirect_blocks.find(lvl2_indirect_bid)?;
                self.indirect_block = Some(lvl2_indirect_block.clone())
            }
        }

        Ok(())
    }
}

impl<'a> Iterator for DeviceRangeReader<'a> {
    type Item = Range<Ext2Bid>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.range.is_empty() {
            return None;
        }

        let range = self.read().unwrap();
        Some(range)
    }
}

impl InodeImpl {
    pub fn new(desc: Dirty<InodeDesc>, weak_self: Weak<Inode>, fs: Weak<Ext2>) -> Arc<Self> {
        let inner = InodeImpl_::new(desc, weak_self, fs);
        Arc::new(Self(RwMutex::new(inner)))
    }

    pub fn file_size(&self) -> usize {
        self.0.read().desc.size
    }

    pub fn resize(&self, new_size: usize) -> Result<()> {
        self.0.write().resize(new_size)
    }

    pub fn file_type(&self) -> FileType {
        self.0.read().desc.type_
    }

    pub fn file_perm(&self) -> FilePerm {
        self.0.read().desc.perm
    }

    pub fn set_file_perm(&self, perm: FilePerm) {
        let mut inner = self.0.write();
        inner.desc.perm = perm;
    }

    pub fn uid(&self) -> u32 {
        self.0.read().desc.uid
    }

    pub fn set_uid(&self, uid: u32) {
        let mut inner = self.0.write();
        inner.desc.uid = uid;
    }

    pub fn gid(&self) -> u32 {
        self.0.read().desc.gid
    }

    pub fn set_gid(&self, gid: u32) {
        let mut inner = self.0.write();
        inner.desc.gid = gid;
    }

    pub fn file_flags(&self) -> FileFlags {
        self.0.read().desc.flags
    }

    pub fn hard_links(&self) -> u16 {
        self.0.read().desc.hard_links
    }

    pub fn inc_hard_links(&self) {
        let mut inner = self.0.write();
        inner.desc.hard_links += 1;
    }

    pub fn dec_hard_links(&self) {
        let mut inner = self.0.write();
        debug_assert!(inner.desc.hard_links > 0);
        inner.desc.hard_links -= 1;
    }

    pub fn blocks_count(&self) -> Ext2Bid {
        self.0.read().desc.blocks_count()
    }

    pub fn acl(&self) -> Option<Bid> {
        self.0.read().desc.acl
    }

    pub fn atime(&self) -> Duration {
        self.0.read().desc.atime
    }

    pub fn set_atime(&self, time: Duration) {
        let mut inner = self.0.write();
        inner.desc.atime = time;
    }

    pub fn mtime(&self) -> Duration {
        self.0.read().desc.mtime
    }

    pub fn set_mtime(&self, time: Duration) {
        let mut inner = self.0.write();
        inner.desc.mtime = time;
    }

    pub fn ctime(&self) -> Duration {
        self.0.read().desc.ctime
    }

    pub fn read_block_sync(&self, bid: Ext2Bid, block: &VmFrame) -> Result<()> {
        self.0.read().read_block_sync(bid, block)
    }

    pub fn read_block_async(&self, bid: Ext2Bid, block: &VmFrame) -> Result<BioWaiter> {
        self.0.read().read_block_async(bid, block)
    }

    pub fn write_block_sync(&self, bid: Ext2Bid, block: &VmFrame) -> Result<()> {
        self.0.read().write_block_sync(bid, block)
    }

    pub fn write_block_async(&self, bid: Ext2Bid, block: &VmFrame) -> Result<BioWaiter> {
        self.0.read().write_block_async(bid, block)
    }

    pub fn set_device_id(&self, device_id: u64) {
        self.0.write().desc.block_ptrs.as_bytes_mut()[..core::mem::size_of::<u64>()]
            .copy_from_slice(device_id.as_bytes());
    }

    pub fn device_id(&self) -> u64 {
        let mut device_id: u64 = 0;
        device_id.as_bytes_mut().copy_from_slice(
            &self.0.read().desc.block_ptrs.as_bytes()[..core::mem::size_of::<u64>()],
        );
        device_id
    }

    pub fn write_link(&self, target: &str) -> Result<()> {
        let mut inner = self.0.write();
        inner.desc.block_ptrs.as_bytes_mut()[..target.len()].copy_from_slice(target.as_bytes());
        if inner.desc.size != target.len() {
            inner.resize(target.len())?;
        }
        Ok(())
    }

    pub fn read_link(&self) -> Result<String> {
        let inner = self.0.read();
        let mut symlink = vec![0u8; inner.desc.size];
        symlink.copy_from_slice(&inner.desc.block_ptrs.as_bytes()[..inner.desc.size]);
        Ok(String::from_utf8(symlink)?)
    }

    pub fn sync_data_holes(&self) -> Result<()> {
        let inner = self.0.read();
        let zero_frame = VmAllocOptions::new(1).alloc_single().unwrap();
        for bid in 0..inner.desc.blocks_count() {
            let is_data_hole = inner.blocks_hole_desc.read().is_hole(bid as usize);
            if is_data_hole {
                inner.write_block_sync(bid, &zero_frame)?;
            }
        }
        Ok(())
    }

    pub fn sync_metadata(&self) -> Result<()> {
        if !self.0.read().desc.is_dirty() {
            return Ok(());
        }

        let mut inner = self.0.write();
        if !inner.desc.is_dirty() {
            return Ok(());
        }

        let inode = inner.inode();
        if inner.desc.hard_links == 0 {
            inner.resize(0)?;
            // Adds the check here to prevent double-free.
            if !inner.is_freed {
                inode
                    .fs()
                    .free_inode(inode.ino(), inner.desc.type_ == FileType::Dir)?;
                inner.is_freed = true;
            }
        }

        inner.indirect_blocks.write().evict_all()?;
        inode.fs().sync_inode(inode.ino(), &inner.desc)?;
        inner.desc.clear_dirty();
        Ok(())
    }
}

impl PageCacheBackend for InodeImpl {
    fn read_page(&self, idx: usize, frame: &VmFrame) -> Result<BioWaiter> {
        let bid = idx as Ext2Bid;
        self.read_block_async(bid, frame)
    }

    fn write_page(&self, idx: usize, frame: &VmFrame) -> Result<BioWaiter> {
        let bid = idx as Ext2Bid;
        self.write_block_async(bid, frame)
    }

    fn npages(&self) -> usize {
        self.blocks_count() as _
    }
}

/// The in-memory rust inode descriptor.
///
/// It represents a file, directory, symbolic link, etc.
/// It contains pointers to the filesystem blocks which contain the data held in the
/// object and all of the metadata about an object except its name.
///
/// Each block group has an inode table it is responsible for.
#[derive(Clone, Copy, Debug)]
pub(super) struct InodeDesc {
    /// Type.
    type_: FileType,
    /// Permission.
    perm: FilePerm,
    /// User Id.
    uid: u32,
    /// Group Id.
    gid: u32,
    /// Size in bytes.
    size: usize,
    /// Access time.
    atime: Duration,
    /// Creation time.
    ctime: Duration,
    /// Modification time.
    mtime: Duration,
    /// Deletion time.
    dtime: Duration,
    /// Hard links count.
    hard_links: u16,
    /// Number of blocks.
    blocks_count: Ext2Bid,
    /// File flags.
    flags: FileFlags,
    /// Pointers to blocks.
    block_ptrs: BlockPtrs,
    /// File or directory acl block.
    acl: Option<Bid>,
}

impl TryFrom<RawInode> for InodeDesc {
    type Error = crate::error::Error;

    fn try_from(inode: RawInode) -> Result<Self> {
        let file_type = FileType::from_raw_mode(inode.mode)?;
        Ok(Self {
            type_: file_type,
            perm: FilePerm::from_raw_mode(inode.mode)?,
            uid: (inode.os_dependent_2.uid_high as u32) << 16 | inode.uid as u32,
            gid: (inode.os_dependent_2.gid_high as u32) << 16 | inode.gid as u32,
            size: if file_type == FileType::File {
                (inode.size_high as usize) << 32 | inode.size_low as usize
            } else {
                inode.size_low as usize
            },
            atime: Duration::from(inode.atime),
            ctime: Duration::from(inode.ctime),
            mtime: Duration::from(inode.mtime),
            dtime: Duration::from(inode.dtime),
            hard_links: inode.hard_links,
            blocks_count: inode.blocks_count,
            flags: FileFlags::from_bits(inode.flags)
                .ok_or(Error::with_message(Errno::EINVAL, "invalid file flags"))?,
            block_ptrs: inode.block_ptrs,
            acl: match file_type {
                FileType::File => Some(Bid::new(inode.file_acl as _)),
                FileType::Dir => Some(Bid::new(inode.size_high as _)),
                _ => None,
            },
        })
    }
}

impl InodeDesc {
    pub fn new(type_: FileType, perm: FilePerm) -> Dirty<Self> {
        Dirty::new_dirty(Self {
            type_,
            perm,
            uid: 0,
            gid: 0,
            size: 0,
            atime: Duration::ZERO,
            ctime: Duration::ZERO,
            mtime: Duration::ZERO,
            dtime: Duration::ZERO,
            hard_links: 1,
            blocks_count: 0,
            flags: FileFlags::empty(),
            block_ptrs: BlockPtrs::default(),
            acl: match type_ {
                FileType::File | FileType::Dir => Some(Bid::new(0)),
                _ => None,
            },
        })
    }

    pub fn num_page_bytes(&self) -> usize {
        (self.blocks_count() as usize) * BLOCK_SIZE
    }

    /// Returns the actual number of blocks utilized.
    ///
    /// Ext2 allows the `block_count` to exceed the actual number of blocks utilized.
    pub fn blocks_count(&self) -> Ext2Bid {
        let blocks = self.size_to_blocks(self.size);
        assert!(blocks <= self.blocks_count);
        blocks
    }

    #[inline]
    fn size_to_blocks(&self, size: usize) -> Ext2Bid {
        if self.type_ == FileType::Symlink && size <= MAX_FAST_SYMLINK_LEN {
            return 0;
        }
        size.div_ceil(BLOCK_SIZE) as Ext2Bid
    }
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
    pub fn from_raw_mode(mode: u16) -> Result<Self> {
        const TYPE_MASK: u16 = 0o170000;
        Self::try_from(mode & TYPE_MASK)
            .map_err(|_| Error::with_message(Errno::EINVAL, "invalid file type"))
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
    pub fn from_raw_mode(mode: u16) -> Result<Self> {
        const PERM_MASK: u16 = 0o7777;
        Self::from_bits(mode & PERM_MASK)
            .ok_or(Error::with_message(Errno::EINVAL, "invalid file perm"))
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

const_assert!(core::mem::size_of::<RawInode>() == 128);

/// The raw inode on device.
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
    pub atime: UnixTime,
    /// Creation time.
    pub ctime: UnixTime,
    /// Modification time.
    pub mtime: UnixTime,
    /// Deletion time.
    pub dtime: UnixTime,
    /// Low 16 bits of Group Id.
    pub gid: u16,
    pub hard_links: u16,
    pub blocks_count: u32,
    /// File flags.
    pub flags: u32,
    /// OS dependent Value 1.
    reserved1: u32,
    /// Pointers to blocks.
    pub block_ptrs: BlockPtrs,
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

impl From<&InodeDesc> for RawInode {
    fn from(inode: &InodeDesc) -> Self {
        Self {
            mode: inode.type_ as u16 | inode.perm.bits(),
            uid: inode.uid as u16,
            size_low: inode.size as u32,
            atime: UnixTime::from(inode.atime),
            ctime: UnixTime::from(inode.ctime),
            mtime: UnixTime::from(inode.mtime),
            dtime: UnixTime::from(inode.dtime),
            gid: inode.gid as u16,
            hard_links: inode.hard_links,
            blocks_count: inode.blocks_count,
            flags: inode.flags.bits(),
            block_ptrs: inode.block_ptrs,
            file_acl: match inode.acl {
                Some(acl) if inode.type_ == FileType::File => acl.to_raw() as u32,
                _ => Default::default(),
            },
            size_high: match inode.acl {
                Some(acl) if inode.type_ == FileType::Dir => acl.to_raw() as u32,
                _ => Default::default(),
            },
            os_dependent_2: Osd2 {
                uid_high: (inode.uid >> 16) as u16,
                gid_high: (inode.gid >> 16) as u16,
                ..Default::default()
            },
            ..Default::default()
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

fn is_block_aligned(offset: usize) -> bool {
    offset % BLOCK_SIZE == 0
}
