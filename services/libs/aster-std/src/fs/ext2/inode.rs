// SPDX-License-Identifier: MPL-2.0

use super::blocks_hole::BlocksHoleDesc;
use super::dir::{DirEntry, DirEntryReader, DirEntryWriter};
use super::fs::Ext2;
use super::prelude::*;

use core::cmp::Ordering;
use inherit_methods_macro::inherit_methods;

mod field {
    pub type Field = core::ops::Range<usize>;

    /// Direct pointer to blocks.
    pub const DIRECT: Field = 0..12;
    /// Indirect pointer to blocks.
    pub const INDIRECT: Field = 12..13;
    /// Doubly indirect pointer to blocks.
    pub const DB_INDIRECT: Field = 13..14;
    /// Trebly indirect pointer to blocks.
    pub const TB_INDIRECT: Field = 14..15;
}

/// The number of block pointers.
pub const BLOCK_PTR_CNT: usize = field::TB_INDIRECT.end;
/// Max length of file name.
pub const MAX_FNAME_LEN: usize = 255;
/// Max path length of the fast symlink.
pub const FAST_SYMLINK_MAX_LEN: usize = BLOCK_PTR_CNT * core::mem::size_of::<u32>();

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
            inner: RwMutex::new(Inner::new(desc, weak_self.clone())),
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
    pub fn blocks_count(&self) -> u32;
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
    pub fn gid(&self) -> u32;
    pub fn file_flags(&self) -> FileFlags;
    pub fn hard_links(&self) -> u16;
    pub fn inc_hard_links(&mut self);
    pub fn dec_hard_links(&mut self);
    pub fn blocks_count(&self) -> u32;
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
    pub fn new(desc: Dirty<InodeDesc>, weak_self: Weak<Inode>) -> Self {
        let num_page_bytes = desc.num_page_bytes();
        let inode_impl = InodeImpl::new(desc, weak_self);
        Self {
            page_cache: PageCache::with_capacity(num_page_bytes, Arc::downgrade(&inode_impl) as _)
                .unwrap(),
            inode_impl,
        }
    }

    pub fn resize(&mut self, new_size: usize) -> Result<()> {
        self.page_cache.pages().resize(new_size)?;
        self.inode_impl.resize(new_size)?;
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
            self.inode_impl.read_block(bid, &frame)?;
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
            self.inode_impl.write_block(bid, &frame)?;
            buf_offset += BLOCK_SIZE;
        }

        Ok(())
    }

    pub fn write_link(&mut self, target: &str) -> Result<()> {
        if target.len() <= FAST_SYMLINK_MAX_LEN {
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
        if file_size <= FAST_SYMLINK_MAX_LEN {
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
    blocks_hole_desc: BlocksHoleDesc,
    is_freed: bool,
    weak_self: Weak<Inode>,
}

impl InodeImpl_ {
    pub fn new(desc: Dirty<InodeDesc>, weak_self: Weak<Inode>) -> Self {
        Self {
            blocks_hole_desc: BlocksHoleDesc::new(desc.blocks_count() as usize),
            desc,
            is_freed: false,
            weak_self,
        }
    }

    pub fn inode(&self) -> Arc<Inode> {
        self.weak_self.upgrade().unwrap()
    }

    pub fn read_block(&self, bid: Bid, block: &VmFrame) -> Result<()> {
        let bid = bid.to_raw() as u32;
        if bid >= self.desc.blocks_count() {
            return_errno!(Errno::EINVAL);
        }

        debug_assert!(field::DIRECT.contains(&(bid as usize)));
        if self.blocks_hole_desc.is_hole(bid as usize) {
            block.zero();
            return Ok(());
        }
        let device_bid = Bid::new(self.desc.data[bid as usize] as _);
        self.inode().fs().read_block(device_bid, block)?;
        Ok(())
    }

    pub fn write_block(&self, bid: Bid, block: &VmFrame) -> Result<()> {
        let bid = bid.to_raw() as u32;
        if bid >= self.desc.blocks_count() {
            return_errno!(Errno::EINVAL);
        }

        debug_assert!(field::DIRECT.contains(&(bid as usize)));
        let device_bid = Bid::new(self.desc.data[bid as usize] as _);
        self.inode().fs().write_block(device_bid, block)?;
        Ok(())
    }

    pub fn resize(&mut self, new_size: usize) -> Result<()> {
        let new_blocks = if self.desc.type_ == FileType::Symlink && new_size <= FAST_SYMLINK_MAX_LEN
        {
            0
        } else {
            new_size.div_ceil(BLOCK_SIZE) as u32
        };
        let old_blocks = self.desc.blocks_count();

        match new_blocks.cmp(&old_blocks) {
            Ordering::Greater => {
                // Allocate blocks
                for file_bid in old_blocks..new_blocks {
                    debug_assert!(field::DIRECT.contains(&(file_bid as usize)));
                    let device_bid = self
                        .inode()
                        .fs()
                        .alloc_block(self.inode().block_group_idx)?;
                    self.desc.data[file_bid as usize] = device_bid.to_raw() as u32;
                }
                self.desc.blocks_count = new_blocks;
            }
            Ordering::Equal => (),
            Ordering::Less => {
                // Free blocks
                for file_bid in new_blocks..old_blocks {
                    debug_assert!(field::DIRECT.contains(&(file_bid as usize)));
                    let device_bid = Bid::new(self.desc.data[file_bid as usize] as _);
                    self.inode().fs().free_block(device_bid)?;
                }
                self.desc.blocks_count = new_blocks;
            }
        }

        self.desc.size = new_size;
        self.blocks_hole_desc.resize(new_blocks as usize);
        Ok(())
    }
}

impl InodeImpl {
    pub fn new(desc: Dirty<InodeDesc>, weak_self: Weak<Inode>) -> Arc<Self> {
        let inner = InodeImpl_::new(desc, weak_self);
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

    pub fn gid(&self) -> u32 {
        self.0.read().desc.gid
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

    pub fn blocks_count(&self) -> u32 {
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

    pub fn read_block(&self, bid: Bid, block: &VmFrame) -> Result<()> {
        self.0.read().read_block(bid, block)
    }

    pub fn write_block(&self, bid: Bid, block: &VmFrame) -> Result<()> {
        let inner = self.0.read();
        inner.write_block(bid, block)?;

        let bid = bid.to_raw() as usize;
        if inner.blocks_hole_desc.is_hole(bid) {
            drop(inner);
            let mut inner = self.0.write();
            if bid < inner.blocks_hole_desc.size() && inner.blocks_hole_desc.is_hole(bid) {
                inner.blocks_hole_desc.unset(bid);
            }
        }
        Ok(())
    }

    pub fn set_device_id(&self, device_id: u64) {
        self.0.write().desc.data.as_bytes_mut()[..core::mem::size_of::<u64>()]
            .copy_from_slice(device_id.as_bytes());
    }

    pub fn device_id(&self) -> u64 {
        let mut device_id: u64 = 0;
        device_id
            .as_bytes_mut()
            .copy_from_slice(&self.0.read().desc.data.as_bytes()[..core::mem::size_of::<u64>()]);
        device_id
    }

    pub fn write_link(&self, target: &str) -> Result<()> {
        let mut inner = self.0.write();
        inner.desc.data.as_bytes_mut()[..target.len()].copy_from_slice(target.as_bytes());
        if inner.desc.size != target.len() {
            inner.resize(target.len())?;
        }
        Ok(())
    }

    pub fn read_link(&self) -> Result<String> {
        let inner = self.0.read();
        let mut symlink = vec![0u8; inner.desc.size];
        symlink.copy_from_slice(&inner.desc.data.as_bytes()[..inner.desc.size]);
        Ok(String::from_utf8(symlink)?)
    }

    pub fn sync_data_holes(&self) -> Result<()> {
        let mut inner = self.0.write();
        let zero_frame = VmAllocOptions::new(1).alloc_single().unwrap();
        for bid in 0..inner.desc.blocks_count() {
            if inner.blocks_hole_desc.is_hole(bid as usize) {
                inner.write_block(Bid::new(bid as _), &zero_frame)?;
                inner.blocks_hole_desc.unset(bid as usize);
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

        inode.fs().sync_inode(inode.ino(), &inner.desc)?;
        inner.desc.clear_dirty();
        Ok(())
    }
}

impl PageCacheBackend for InodeImpl {
    fn read_page(&self, idx: usize, frame: &VmFrame) -> Result<()> {
        let bid = Bid::new(idx as _);
        self.read_block(bid, frame)?;
        Ok(())
    }

    fn write_page(&self, idx: usize, frame: &VmFrame) -> Result<()> {
        let bid = Bid::new(idx as _);
        self.write_block(bid, frame)?;
        Ok(())
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
    blocks_count: u32,
    /// File flags.
    flags: FileFlags,
    /// Pointers to blocks.
    data: [u32; BLOCK_PTR_CNT],
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
            data: inode.data,
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
            data: [0; BLOCK_PTR_CNT],
            acl: match type_ {
                FileType::File | FileType::Dir => Some(Bid::new(0)),
                _ => None,
            },
        })
    }

    pub fn num_page_bytes(&self) -> usize {
        (self.blocks_count() as usize) * BLOCK_SIZE
    }

    pub fn blocks_count(&self) -> u32 {
        if self.type_ == FileType::Dir {
            let real_blocks = (self.size / BLOCK_SIZE) as u32;
            assert!(real_blocks <= self.blocks_count);
            return real_blocks;
        }
        self.blocks_count
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
    pub data: [u32; BLOCK_PTR_CNT],
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
            data: inode.data,
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
