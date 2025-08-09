// SPDX-License-Identifier: MPL-2.0

#![expect(dead_code)]
#![expect(unused_variables)]

use alloc::{borrow::ToOwned, rc::Rc};
use core::sync::atomic::{AtomicUsize, Ordering};

use inherit_methods_macro::inherit_methods;
use ostd::{const_assert, mm::io_util::HasVmReaderWriter};

use super::{
    block_ptr::{BidPath, BlockPtrs, Ext2Bid, BID_SIZE, MAX_BLOCK_PTRS},
    dir::{DirEntryHeader, DirEntryItem, DirEntryReader, DirEntryWriter},
    fs::Ext2,
    indirect_block_cache::{IndirectBlock, IndirectBlockCache},
    prelude::*,
    utils::now,
    xattr::Xattr,
};
use crate::{
    fs::{
        path::{is_dot, is_dot_or_dotdot, is_dotdot},
        utils::{
            Extension, FallocMode, Inode as _, InodeMode, Metadata, Permission, XattrName,
            XattrNamespace, XattrSetFlags,
        },
    },
    process::{posix_thread::AsPosixThread, Gid, Uid},
};

/// Max length of file name.
pub const MAX_FNAME_LEN: usize = 255;

/// Max path length of the fast symlink.
pub const MAX_FAST_SYMLINK_LEN: usize = MAX_BLOCK_PTRS * BID_SIZE;

/// The Ext2 inode.
pub struct Inode {
    ino: u32,
    type_: InodeType,
    block_group_idx: usize,
    inner: RwMutex<InodeInner>,
    fs: Weak<Ext2>,
    extension: Extension,
    xattr: Option<Xattr>,
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
            type_: desc.type_,
            block_group_idx,
            xattr: desc
                .acl
                .map(|acl| Xattr::new(acl, weak_self.clone(), fs.clone())),
            inner: RwMutex::new(InodeInner::new(desc, weak_self.clone(), fs.clone())),
            fs,
            extension: Extension::new(),
        })
    }

    pub fn ino(&self) -> u32 {
        self.ino
    }

    pub fn inode_type(&self) -> InodeType {
        self.type_
    }

    pub(super) fn block_group_idx(&self) -> usize {
        self.block_group_idx
    }

    pub fn fs(&self) -> Arc<Ext2> {
        self.fs.upgrade().unwrap()
    }

    pub fn page_cache(&self) -> Vmo<Full> {
        self.inner.read().page_cache.pages().dup()
    }

    pub fn metadata(&self) -> Metadata {
        let inner = self.inner.read();
        Metadata {
            dev: 0, // TODO: ID of block device
            ino: self.ino() as _,
            size: inner.file_size() as _,
            blk_size: BLOCK_SIZE,
            blocks: inner.blocks_count() as _,
            atime: inner.atime(),
            mtime: inner.mtime(),
            ctime: inner.ctime(),
            type_: self.type_,
            mode: InodeMode::from(inner.file_perm()),
            nlinks: inner.hard_links() as _,
            uid: Uid::new(inner.uid()),
            gid: Gid::new(inner.gid()),
            rdev: self.device_id(),
        }
    }

    pub fn resize(&self, new_size: usize) -> Result<()> {
        if self.type_ != InodeType::File {
            return_errno!(Errno::EISDIR);
        }

        let inner = self.inner.upread();
        if new_size == inner.file_size() {
            return Ok(());
        }

        let mut inner = inner.upgrade();
        inner.resize(new_size)?;

        let now = now();
        inner.set_mtime(now);
        inner.set_ctime(now);

        Ok(())
    }

    pub fn create(
        &self,
        name: &str,
        inode_type: InodeType,
        file_perm: FilePerm,
    ) -> Result<Arc<Self>> {
        if name.len() > MAX_FNAME_LEN {
            return_errno!(Errno::ENAMETOOLONG);
        }
        if self.type_ != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let inner = self.inner.upread();
        if inner.hard_links() == 0 {
            return_errno_with_message!(Errno::ENOENT, "dir removed");
        }

        let inode = self
            .fs()
            .create_inode(self.block_group_idx, inode_type, file_perm)?;
        let is_dir = inode_type == InodeType::Dir;
        if let Err(e) = inode.init(self.ino) {
            self.fs().free_inode(inode.ino, is_dir).unwrap();
            return Err(e);
        }

        let mut inner = inner.upgrade();
        if let Err(e) = inner.append_new_entry(inode.ino, inode_type, name, true) {
            self.fs().free_inode(inode.ino, is_dir).unwrap();
            return Err(e);
        }

        let now = now();
        inner.set_mtime(now);
        inner.set_ctime(now);

        Ok(inode)
    }

    fn init(&self, dir_ino: u32) -> Result<()> {
        match self.type_ {
            InodeType::Dir => {
                self.inner.write().init_dir(self.ino, dir_ino)?;
            }
            _ => {
                // TODO: Reserve serval blocks for regular file?
            }
        }
        Ok(())
    }

    pub fn lookup(&self, name: &str) -> Result<Arc<Self>> {
        if name.len() > MAX_FNAME_LEN {
            return_errno!(Errno::ENAMETOOLONG);
        }
        if self.type_ != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let inner = self.inner.read();
        if inner.hard_links() == 0 {
            return_errno_with_message!(Errno::ENOENT, "dir removed");
        }

        let ino = inner
            .find_entry_item(name)
            .map(|entry| entry.ino())
            .ok_or(Error::new(Errno::ENOENT))?;
        drop(inner);
        self.fs().lookup_inode(ino)
    }

    pub fn link(&self, inode: &Inode, name: &str) -> Result<()> {
        if name.len() > MAX_FNAME_LEN {
            return_errno!(Errno::ENAMETOOLONG);
        }
        if self.type_ != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let inner = self.inner.upread();
        if inner.hard_links() == 0 {
            return_errno_with_message!(Errno::ENOENT, "dir removed");
        }

        let inode_type = inode.inode_type();
        if inode_type == InodeType::Dir {
            return_errno!(Errno::EPERM);
        }

        let mut inner = inner.upgrade();
        inner.append_new_entry(inode.ino, inode_type, name, true)?;
        let now = now();
        inner.set_mtime(now);
        inner.set_ctime(now);
        drop(inner);

        let mut inode_inner = inode.inner.write();
        inode_inner.inc_hard_links();
        inode_inner.set_ctime(now);

        Ok(())
    }

    pub fn unlink(&self, name: &str) -> Result<()> {
        if is_dot_or_dotdot(name) {
            return_errno!(Errno::EISDIR);
        }

        let file = self.lookup(name)?;
        if file.inode_type() == InodeType::Dir {
            return_errno!(Errno::EISDIR);
        }

        let (mut self_inner, mut file_inner) = write_lock_two_inodes(self, &file);
        // When we got the lock, the dir may have been modified by another thread
        if self_inner.hard_links() == 0 {
            return_errno_with_message!(Errno::ENOENT, "dir removed");
        }
        let (offset, new_ino) = self_inner
            .find_entry_item(name)
            .map(|entry| (entry.offset(), entry.ino()))
            .ok_or(Error::new(Errno::ENOENT))?;
        if file.ino != new_ino {
            return_errno!(Errno::ENOENT);
        }
        let potential_new_file = self.fs().lookup_inode(file.ino)?;
        if !Arc::ptr_eq(&file, &potential_new_file) {
            return_errno!(Errno::ENOENT);
        }

        self_inner.remove_entry_at(name, offset)?;
        file_inner.dec_hard_links();
        let now = now();
        self_inner.set_mtime(now);
        self_inner.set_ctime(now);
        file_inner.set_ctime(now);

        Ok(())
    }

    pub fn rmdir(&self, name: &str) -> Result<()> {
        if is_dot(name) {
            return_errno_with_message!(Errno::EINVAL, "rmdir on .");
        }
        if is_dotdot(name) {
            return_errno_with_message!(Errno::ENOTEMPTY, "rmdir on ..");
        }

        let dir_inode = self.lookup(name)?;
        let dir_inner = dir_inode.inner.read();
        if dir_inner.inode_type() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if dir_inner.entry_count() > 2 {
            return_errno!(Errno::ENOTEMPTY);
        }
        drop(dir_inner);

        let (mut self_inner, mut dir_inner) = write_lock_two_inodes(self, &dir_inode);
        // When we got the lock, the dir may have been modified by another thread
        if self_inner.hard_links() == 0 {
            return_errno_with_message!(Errno::ENOENT, "dir removed");
        }
        let (offset, new_ino) = self_inner
            .find_entry_item(name)
            .map(|entry| (entry.offset(), entry.ino()))
            .ok_or(Error::new(Errno::ENOENT))?;
        if dir_inode.ino != new_ino {
            return_errno!(Errno::ENOENT);
        }
        let potential_new_dir = self.fs().lookup_inode(dir_inode.ino)?;
        if !Arc::ptr_eq(&dir_inode, &potential_new_dir) {
            return_errno!(Errno::ENOENT);
        }
        if dir_inner.inode_type() != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }
        if dir_inner.entry_count() > 2 {
            return_errno!(Errno::ENOTEMPTY);
        }

        self_inner.remove_entry_at(name, offset)?;
        let now = now();
        self_inner.set_mtime(now);
        self_inner.set_ctime(now);
        dir_inner.dec_hard_links();
        dir_inner.dec_hard_links(); // For "."

        Ok(())
    }

    /// Rename within its own directory.
    fn rename_within(&self, old_name: &str, new_name: &str) -> Result<()> {
        if self.type_ != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let self_inner = self.inner.upread();
        if self_inner.hard_links() == 0 {
            return_errno_with_message!(Errno::ENOENT, "dir removed");
        }

        let fs = self.fs();
        let (src_offset, src_inode, src_inode_typ) = {
            let entry = self_inner
                .find_entry_item(old_name)
                .ok_or(Error::new(Errno::ENOENT))?;
            (entry.offset(), fs.lookup_inode(entry.ino())?, entry.type_())
        };

        let Some(dst_ino) = self_inner
            .find_entry_item(new_name)
            .map(|entry| entry.ino())
        else {
            let mut self_inner = self_inner.upgrade();
            self_inner.rename_entry_at(old_name, new_name, src_offset)?;
            let now = now();
            self_inner.set_mtime(now);
            self_inner.set_ctime(now);
            drop(self_inner);

            src_inode.set_ctime(now);
            return Ok(());
        };
        if src_inode.ino == dst_ino {
            // Same inode, do nothing
            return Ok(());
        }
        let dst_inode = fs.lookup_inode(dst_ino)?;
        drop(self_inner);

        let (mut self_inner, mut dst_inner) = write_lock_two_inodes(self, &dst_inode);
        // When we got the lock, the dir may have been modified by another thread
        if self_inner.hard_links() == 0 {
            return_errno_with_message!(Errno::ENOENT, "dir removed");
        }

        let (src_offset, new_src_ino) = self_inner
            .find_entry_item(old_name)
            .map(|entry| (entry.offset(), entry.ino()))
            .ok_or(Error::new(Errno::ENOENT))?;
        if src_inode.ino != new_src_ino {
            return_errno!(Errno::ENOENT);
        }
        let potential_new_src = fs.lookup_inode(src_inode.ino)?;
        if !Arc::ptr_eq(&src_inode, &potential_new_src) {
            return_errno!(Errno::ENOENT);
        }

        let new_dst_entry = self_inner
            .find_entry_item(new_name)
            .ok_or(Error::new(Errno::ENOENT))?;
        let dst_offset = new_dst_entry.offset();
        if dst_inode.ino != new_dst_entry.ino() {
            return_errno!(Errno::ENOENT);
        }
        let potential_new_dst = fs.lookup_inode(dst_inode.ino)?;
        if !Arc::ptr_eq(&dst_inode, &potential_new_dst) {
            return_errno!(Errno::ENOENT);
        }

        let dst_inode_typ = new_dst_entry.type_();
        match (src_inode_typ, dst_inode_typ) {
            (InodeType::Dir, InodeType::Dir) => {
                if dst_inner.entry_count() > 2 {
                    return_errno!(Errno::ENOTEMPTY);
                }
            }
            (InodeType::Dir, _) => {
                return_errno!(Errno::ENOTDIR);
            }
            (_, InodeType::Dir) => {
                return_errno!(Errno::EISDIR);
            }
            _ => {}
        }

        self_inner.remove_entry_at(new_name, dst_offset)?;
        self_inner.rename_entry_at(old_name, new_name, src_offset)?;
        let now = now();
        self_inner.set_mtime(now);
        self_inner.set_ctime(now);

        dst_inner.dec_hard_links();
        if dst_inode_typ == InodeType::Dir {
            dst_inner.dec_hard_links(); // For "."
        }
        dst_inner.set_ctime(now);
        drop(self_inner);
        drop(dst_inner);

        src_inode.set_ctime(now);

        Ok(())
    }

    pub fn rename(&self, old_name: &str, target: &Inode, new_name: &str) -> Result<()> {
        if is_dot_or_dotdot(old_name) || is_dot_or_dotdot(new_name) {
            return_errno!(Errno::EISDIR);
        }
        if old_name.len() > MAX_FNAME_LEN || new_name.len() > MAX_FNAME_LEN {
            return_errno!(Errno::ENAMETOOLONG);
        }

        // Rename inside the inode
        if self.ino == target.ino {
            return self.rename_within(old_name, new_name);
        }

        let (self_inner, target_inner) = read_lock_two_inodes(self, target);
        if self_inner.inode_type() != InodeType::Dir || target_inner.inode_type() != InodeType::Dir
        {
            return_errno!(Errno::ENOTDIR);
        }
        if self_inner.hard_links() == 0 || target_inner.hard_links() == 0 {
            return_errno_with_message!(Errno::ENOENT, "dir removed");
        }

        let fs = self.fs();
        let (src_offset, src_inode, src_inode_typ) = {
            let entry = self_inner
                .find_entry_item(old_name)
                .ok_or(Error::new(Errno::ENOENT))?;
            (entry.offset(), fs.lookup_inode(entry.ino())?, entry.type_())
        };
        // Avoid renaming a directory to a subdirectory of itself
        if src_inode.ino == target.ino {
            return_errno!(Errno::EINVAL);
        }
        let is_dir = src_inode_typ == InodeType::Dir;

        let Some(dst_ino) = target_inner
            .find_entry_item(new_name)
            .map(|entry| entry.ino())
        else {
            drop(self_inner);
            drop(target_inner);

            let mut write_guards = if is_dir {
                write_lock_multiple_inodes(vec![&src_inode, target, self])
            } else {
                write_lock_multiple_inodes(vec![target, self])
            };

            // When we got the lock, the dir may have been modified by another thread
            let mut self_inner = write_guards.pop().unwrap();
            let mut target_inner = write_guards.pop().unwrap();
            if self_inner.hard_links() == 0 || target_inner.hard_links() == 0 {
                return_errno_with_message!(Errno::ENOENT, "dir removed");
            }
            let (src_offset, new_src_ino) = self_inner
                .find_entry_item(old_name)
                .map(|entry| (entry.offset(), entry.ino()))
                .ok_or(Error::new(Errno::ENOENT))?;
            if src_inode.ino != new_src_ino {
                return_errno!(Errno::ENOENT);
            }
            let potential_new_src = fs.lookup_inode(src_inode.ino)?;
            if !Arc::ptr_eq(&src_inode, &potential_new_src) {
                return_errno!(Errno::ENOENT);
            }

            self_inner.remove_entry_at(old_name, src_offset)?;
            target_inner.append_new_entry(src_inode.ino, src_inode_typ, new_name, false)?;
            let now = now();
            self_inner.set_mtime(now);
            self_inner.set_ctime(now);
            target_inner.set_mtime(now);
            target_inner.set_ctime(now);

            if is_dir {
                let mut src_inner = write_guards.pop().unwrap();
                src_inner.set_parent_ino(target.ino)?;
                src_inner.set_ctime(now);
            } else {
                drop(self_inner);
                drop(target_inner);
                src_inode.set_ctime(now);
            }

            return Ok(());
        };
        if src_inode.ino == dst_ino {
            // Same inode, do nothing
            return Ok(());
        }
        // Avoid renaming a subdirectory to a directory.
        if self.ino == dst_ino {
            return_errno!(Errno::ENOTEMPTY);
        }
        let dst_inode = fs.lookup_inode(dst_ino)?;
        drop(self_inner);
        drop(target_inner);

        let mut write_guards = if is_dir {
            write_lock_multiple_inodes(vec![&src_inode, &dst_inode, target, self])
        } else {
            write_lock_multiple_inodes(vec![&dst_inode, target, self])
        };

        // When we got the lock, the dir may have been modified by another thread
        let mut self_inner = write_guards.pop().unwrap();
        let mut target_inner = write_guards.pop().unwrap();
        if self_inner.hard_links() == 0 || target_inner.hard_links() == 0 {
            return_errno_with_message!(Errno::ENOENT, "dir removed");
        }

        let (src_offset, new_src_ino) = self_inner
            .find_entry_item(old_name)
            .map(|entry| (entry.offset(), entry.ino()))
            .ok_or(Error::new(Errno::ENOENT))?;
        if src_inode.ino != new_src_ino {
            return_errno!(Errno::ENOENT);
        }
        let potential_new_src = fs.lookup_inode(src_inode.ino)?;
        if !Arc::ptr_eq(&src_inode, &potential_new_src) {
            return_errno!(Errno::ENOENT);
        }

        let new_dst_entry = target_inner
            .find_entry_item(new_name)
            .ok_or(Error::new(Errno::ENOENT))?;
        let dst_offset = new_dst_entry.offset();
        if dst_inode.ino != new_dst_entry.ino() {
            return_errno!(Errno::ENOENT);
        }
        let potential_new_dst = fs.lookup_inode(dst_inode.ino)?;
        if !Arc::ptr_eq(&dst_inode, &potential_new_dst) {
            return_errno!(Errno::ENOENT);
        }

        let mut dst_inner = write_guards.pop().unwrap();
        let dst_inode_typ = new_dst_entry.type_();
        match (src_inode_typ, dst_inode_typ) {
            (InodeType::Dir, InodeType::Dir) => {
                if dst_inner.entry_count() > 2 {
                    return_errno!(Errno::ENOTEMPTY);
                }
            }
            (InodeType::Dir, _) => {
                return_errno!(Errno::ENOTDIR);
            }
            (_, InodeType::Dir) => {
                return_errno!(Errno::EISDIR);
            }
            _ => {}
        }

        self_inner.remove_entry_at(old_name, src_offset)?;
        target_inner.remove_entry_at(new_name, dst_offset)?;
        target_inner.append_new_entry(src_inode.ino, src_inode_typ, new_name, false)?;
        dst_inner.dec_hard_links();
        let now = now();
        self_inner.set_mtime(now);
        self_inner.set_ctime(now);
        target_inner.set_mtime(now);
        target_inner.set_ctime(now);
        dst_inner.set_ctime(now);

        if is_dir {
            dst_inner.dec_hard_links(); // For "."
            let mut src_inner = write_guards.pop().unwrap();
            src_inner.set_parent_ino(target.ino)?;
            src_inner.set_ctime(now);
        } else {
            drop(self_inner);
            drop(target_inner);
            drop(dst_inner);
            src_inode.set_ctime(now);
        }

        Ok(())
    }

    pub fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        if self.type_ != InodeType::Dir {
            return_errno!(Errno::ENOTDIR);
        }

        let offset_read = {
            let inner = self.inner.read();
            if inner.hard_links() == 0 {
                return_errno_with_message!(Errno::ENOENT, "dir removed");
            }

            let try_readdir = |offset: &mut usize, visitor: &mut dyn DirentVisitor| -> Result<()> {
                let mut dir_entry_reader = DirEntryReader::new(&inner.page_cache, *offset);
                for dir_entry in dir_entry_reader.iter_entries() {
                    visitor.visit(
                        dir_entry.name(),
                        dir_entry.ino() as u64,
                        dir_entry.type_(),
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
            }?
        };

        self.set_atime(now());

        Ok(offset_read)
    }

    pub fn write_link(&self, target: &str) -> Result<()> {
        if self.type_ != InodeType::SymLink {
            return_errno!(Errno::EISDIR);
        }

        let mut inner = self.inner.write();
        inner.write_link(target)
    }

    pub fn read_link(&self) -> Result<String> {
        if self.type_ != InodeType::SymLink {
            return_errno_with_message!(Errno::EINVAL, "self is not symlink");
        }

        let inner = self.inner.read();
        inner.read_link()
    }

    pub fn set_device_id(&self, device_id: u64) -> Result<()> {
        if self.type_ != InodeType::BlockDevice && self.type_ != InodeType::CharDevice {
            return_errno!(Errno::EISDIR);
        }

        let mut inner = self.inner.write();
        inner.set_device_id(device_id);
        Ok(())
    }

    pub fn device_id(&self) -> u64 {
        if self.type_ != InodeType::BlockDevice && self.type_ != InodeType::CharDevice {
            return 0;
        }

        let inner = self.inner.read();
        inner.device_id()
    }

    pub fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        if self.type_ != InodeType::File {
            return_errno!(Errno::EISDIR);
        }

        let bytes_read = self.inner.read().read_at(offset, writer)?;

        self.set_atime(now());

        Ok(bytes_read)
    }

    // The offset and the length of buffer must be multiples of the block size.
    pub fn read_direct_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        if self.type_ != InodeType::File {
            return_errno!(Errno::EISDIR);
        }
        if !is_block_aligned(offset) || !is_block_aligned(writer.avail()) {
            return_errno_with_message!(Errno::EINVAL, "not block-aligned");
        }

        let bytes_read = self.inner.read().read_direct_at(offset, writer)?;

        self.set_atime(now());

        Ok(bytes_read)
    }

    pub fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        if self.type_ != InodeType::File {
            return_errno!(Errno::EISDIR);
        }

        let inner = self.inner.upread();
        let file_size = inner.file_size();
        let new_size = offset + reader.remain();

        let (bytes_written, mut upgraded_inner) = if new_size > file_size {
            let mut inner = inner.upgrade();
            let len = inner.extend_write_at(offset, reader)?;
            (len, inner)
        } else {
            let len = inner.write_at(offset, reader)?;
            (len, inner.upgrade())
        };

        let now = now();
        upgraded_inner.set_mtime(now);
        upgraded_inner.set_ctime(now);

        Ok(bytes_written)
    }

    // The offset and the length of buffer must be multiples of the block size.
    pub fn write_direct_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        if self.type_ != InodeType::File {
            return_errno!(Errno::EISDIR);
        }
        if !is_block_aligned(offset) || !is_block_aligned(reader.remain()) {
            return_errno_with_message!(Errno::EINVAL, "not block aligned");
        }

        let mut inner = self.inner.write();
        let bytes_written = inner.write_direct_at(offset, reader)?;

        let now = now();
        inner.set_mtime(now);
        inner.set_ctime(now);

        Ok(bytes_written)
    }

    pub fn sync_all(&self) -> Result<()> {
        let mut inner = self.inner.write();
        inner.sync_data()?;
        inner.sync_metadata()?;
        if let Some(xattr) = self.xattr.as_ref() {
            xattr.flush()?;
        }
        Ok(())
    }

    pub fn set_file_perm(&self, perm: FilePerm) {
        let mut inner = self.inner.write();
        inner.set_file_perm(perm);
        inner.set_ctime(now());
    }

    pub fn set_uid(&self, uid: u32) {
        let mut inner = self.inner.write();
        inner.set_uid(uid);
        inner.set_ctime(now());
    }

    pub fn set_gid(&self, gid: u32) {
        let mut inner = self.inner.write();
        inner.set_gid(gid);
        inner.set_ctime(now());
    }

    pub fn extension(&self) -> &Extension {
        &self.extension
    }

    pub fn fallocate(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()> {
        if self.type_ != InodeType::File {
            return_errno_with_message!(Errno::EISDIR, "not regular file");
        }

        match mode {
            FallocMode::PunchHoleKeepSize => {
                // Make the whole operation atomic
                let inner = self.inner.write();

                let file_size = inner.file_size();
                if offset >= file_size {
                    return Ok(());
                }
                let end_offset = file_size.min(offset + len);

                // TODO: Think of a more light-weight approach
                inner.page_cache.fill_zeros(offset..end_offset)?;
                Ok(())
            }
            // We extend the compatibility here since Ext2 in Linux
            // does not natively support `Allocate` and `AllocateKeepSize`.
            FallocMode::Allocate => {
                let new_size = offset + len;
                if new_size > self.file_size() {
                    self.resize(new_size)?;
                }
                Ok(())
            }
            FallocMode::AllocateKeepSize => Ok(()),
            _ => {
                return_errno_with_message!(
                    Errno::EOPNOTSUPP,
                    "fallocate with the specified flags is not supported"
                );
            }
        }
    }

    pub fn set_xattr(
        &self,
        name: XattrName,
        value_reader: &mut VmReader,
        flags: XattrSetFlags,
    ) -> Result<()> {
        let xattr = self.xattr.as_ref().ok_or(Error::with_message(
            Errno::EPERM,
            "xattr is not supported on the file type",
        ))?;
        self.check_permission(Permission::MAY_WRITE)?;
        xattr.set(name, value_reader, flags)
    }

    pub fn get_xattr(&self, name: XattrName, value_writer: &mut VmWriter) -> Result<usize> {
        if self.xattr.is_none() {
            return_errno_with_message!(Errno::ENODATA, "no available xattrs");
        }
        self.check_permission(Permission::MAY_READ)?;
        self.xattr.as_ref().unwrap().get(name, value_writer)
    }

    pub fn list_xattr(
        &self,
        namespace: XattrNamespace,
        list_writer: &mut VmWriter,
    ) -> Result<usize> {
        if self.xattr.is_none() || self.check_permission(Permission::MAY_ACCESS).is_err() {
            return Ok(0);
        }
        self.xattr.as_ref().unwrap().list(namespace, list_writer)
    }

    pub fn remove_xattr(&self, name: XattrName) -> Result<()> {
        let xattr = self.xattr.as_ref().ok_or(Error::with_message(
            Errno::EPERM,
            "xattr is not supported on the file type",
        ))?;
        self.check_permission(Permission::MAY_WRITE)?;
        self.xattr.as_ref().unwrap().remove(name)
    }
}

#[inherit_methods(from = "self.inner.read()")]
impl Inode {
    pub fn file_size(&self) -> usize;
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
}

#[inherit_methods(from = "self.inner.write()")]
impl Inode {
    pub fn set_acl(&self, bid: Bid);
    pub fn set_atime(&self, time: Duration);
    pub fn set_mtime(&self, time: Duration);
    pub fn set_ctime(&self, time: Duration);
    pub fn sync_data(&self) -> Result<()>;
    pub fn sync_metadata(&mut self) -> Result<()>;
}

impl Debug for Inode {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("Inode")
            .field("ino", &self.ino)
            .field("block_group_idx", &self.block_group_idx)
            .finish()
    }
}

fn read_lock_two_inodes<'a>(
    this: &'a Inode,
    other: &'a Inode,
) -> (
    RwMutexReadGuard<'a, InodeInner>,
    RwMutexReadGuard<'a, InodeInner>,
) {
    if this.ino < other.ino {
        let this = this.inner.read();
        let other = other.inner.read();
        (this, other)
    } else {
        let other = other.inner.read();
        let this = this.inner.read();
        (this, other)
    }
}

fn write_lock_two_inodes<'a>(
    this: &'a Inode,
    other: &'a Inode,
) -> (
    RwMutexWriteGuard<'a, InodeInner>,
    RwMutexWriteGuard<'a, InodeInner>,
) {
    if this.ino < other.ino {
        let this = this.inner.write();
        let other = other.inner.write();
        (this, other)
    } else {
        let other = other.inner.write();
        let this = this.inner.write();
        (this, other)
    }
}

fn write_lock_multiple_inodes(inodes: Vec<&Inode>) -> Vec<RwMutexWriteGuard<'_, InodeInner>> {
    // Record the index information of the input
    let mut ordered_inodes: Vec<(usize, &Inode)> = inodes.into_iter().enumerate().collect();
    // Sort in ascending order of ino
    ordered_inodes.sort_unstable_by_key(|&(_, inode)| inode.ino);
    // Acquire the guards in order, and record by the input index.
    // This ensures that the output order is consistent with the input.
    let mut guards = vec![None; ordered_inodes.len()];
    for (original_idx, inode) in ordered_inodes {
        guards[original_idx] = Some(Rc::new(inode.inner.write()));
    }
    guards
        .into_iter()
        .map(|guard| Rc::into_inner(guard.unwrap()).unwrap())
        .collect()
}

struct InodeInner {
    inode_impl: InodeImpl,
    page_cache: PageCache,
}

impl InodeInner {
    pub fn new(desc: Dirty<InodeDesc>, weak_self: Weak<Inode>, fs: Weak<Ext2>) -> Self {
        let num_page_bytes = desc.num_page_bytes();
        let inode_impl = InodeImpl::new(desc, weak_self, fs);
        Self {
            page_cache: PageCache::with_capacity(
                num_page_bytes,
                Arc::downgrade(&inode_impl.block_manager) as _,
            )
            .unwrap(),
            inode_impl,
        }
    }

    pub fn resize(&mut self, new_size: usize) -> Result<()> {
        self.page_cache.resize(new_size)?;
        self.inode_impl.resize(new_size)?;
        Ok(())
    }

    pub fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let (offset, read_len) = {
            let file_size = self.inode_impl.file_size();
            let start = file_size.min(offset);
            let end = file_size.min(offset + writer.avail());
            (start, end - start)
        };

        self.page_cache.pages().read(offset, writer)?;
        Ok(read_len)
    }

    pub fn read_direct_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        debug_assert!(is_block_aligned(offset) && is_block_aligned(writer.avail()));
        let (offset, read_len) = {
            let file_size = self.inode_impl.file_size();
            let start = file_size.min(offset).align_down(BLOCK_SIZE);
            let end = file_size
                .min(offset + writer.avail())
                .align_down(BLOCK_SIZE);
            (start, end - start)
        };
        if read_len == 0 {
            return Ok(read_len);
        }
        self.page_cache.discard_range(offset..offset + read_len);

        let start_bid = Bid::from_offset(offset).to_raw() as Ext2Bid;
        let buf_nblocks = read_len / BLOCK_SIZE;
        self.inode_impl
            .read_blocks(start_bid, buf_nblocks, writer)?;

        Ok(read_len)
    }

    pub fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        let write_len = reader.remain();
        self.page_cache.pages().write(offset, reader)?;
        Ok(write_len)
    }

    pub fn extend_write_at(&mut self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        let write_len = reader.remain();
        let new_size = offset + write_len;
        self.page_cache.resize(new_size.align_up(BLOCK_SIZE))?;
        self.page_cache.pages().write(offset, reader)?;
        self.inode_impl.resize(new_size)?;
        Ok(write_len)
    }

    pub fn write_direct_at(&mut self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        debug_assert!(is_block_aligned(offset) && is_block_aligned(reader.remain()));
        let file_size = self.inode_impl.file_size();
        let write_len = reader.remain();
        let end_offset = offset + write_len;

        let start = offset.min(file_size);
        let end = end_offset.min(file_size);
        self.page_cache.discard_range(start..end);

        if end_offset > file_size {
            self.inode_impl.resize(end_offset)?;
        }

        let start_bid = Bid::from_offset(offset).to_raw() as Ext2Bid;
        let buf_nblocks = write_len / BLOCK_SIZE;
        self.inode_impl
            .write_blocks(start_bid, buf_nblocks, reader)?;

        Ok(write_len)
    }

    pub fn write_link(&mut self, target: &str) -> Result<()> {
        if target.len() <= MAX_FAST_SYMLINK_LEN {
            return self.inode_impl.write_link(target);
        }

        self.page_cache.resize(target.len())?;
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
        debug_assert_eq!(self.inode_type(), InodeType::Dir);
        DirEntryWriter::new(&self.page_cache, 0).init_dir(self_ino, parent_ino)?;
        self.inc_hard_links(); // for ".."
        Ok(())
    }

    pub fn contains_entry(&self, name: &str) -> bool {
        DirEntryReader::new(&self.page_cache, 0).contains_entry(name)
    }

    pub fn find_entry_item(&self, name: &str) -> Option<DirEntryItem> {
        DirEntryReader::new(&self.page_cache, 0).find_entry_item(name)
    }

    pub fn entry_count(&self) -> usize {
        DirEntryReader::new(&self.page_cache, 0).entry_count()
    }

    pub fn append_new_entry(
        &mut self,
        ino: u32,
        inode_type: InodeType,
        name: &str,
        check_existence: bool,
    ) -> Result<()> {
        let entry_header = DirEntryHeader::new(ino, inode_type, name.len());
        DirEntryWriter::new(&self.page_cache, 0).append_new_entry(
            entry_header,
            name,
            check_existence,
        )?;

        let file_size = self.file_size();
        let page_cache_size = self.page_cache.pages().size();
        if page_cache_size > file_size {
            self.inode_impl.resize(page_cache_size)?;
        }

        let is_dir = inode_type == InodeType::Dir;
        let is_parent = name == "..";
        if is_dir && !is_parent {
            self.inc_hard_links(); // for ".."
        }
        Ok(())
    }

    pub fn remove_entry_at(&mut self, name: &str, offset: usize) -> Result<()> {
        let removed_entry = DirEntryWriter::new(&self.page_cache, offset).remove_entry(name)?;
        let file_size = self.file_size();
        let page_cache_size = self.page_cache.pages().size();
        if page_cache_size < file_size {
            self.inode_impl.resize(page_cache_size)?;
        }
        if removed_entry.type_() == InodeType::Dir {
            self.dec_hard_links(); // for ".."
        }
        Ok(())
    }

    pub fn rename_entry_at(&mut self, old_name: &str, new_name: &str, offset: usize) -> Result<()> {
        DirEntryWriter::new(&self.page_cache, offset).rename_entry(old_name, new_name)?;
        let file_size = self.file_size();
        let page_cache_size = self.page_cache.pages().size();
        if page_cache_size != file_size {
            self.inode_impl.resize(page_cache_size)?;
        }
        Ok(())
    }

    pub fn set_parent_ino(&mut self, parent_ino: u32) -> Result<()> {
        let mut entry_item = self.find_entry_item("..").unwrap();
        entry_item.set_ino(parent_ino);
        DirEntryWriter::new(&self.page_cache, entry_item.offset())
            .write_header_only(entry_item.header())?;
        Ok(())
    }

    pub fn sync_data(&self) -> Result<()> {
        // Writes back the data in page cache.
        let file_size = self.file_size();
        self.page_cache.evict_range(0..file_size)?;
        Ok(())
    }
}

#[inherit_methods(from = "self.inode_impl")]
impl InodeInner {
    pub fn file_size(&self) -> usize;
    pub fn inode_type(&self) -> InodeType;
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
    pub fn set_acl(&mut self, bid: Bid);
    pub fn atime(&self) -> Duration;
    pub fn set_atime(&mut self, time: Duration);
    pub fn mtime(&self) -> Duration;
    pub fn set_mtime(&mut self, time: Duration);
    pub fn ctime(&self) -> Duration;
    pub fn set_ctime(&mut self, time: Duration);
    pub fn device_id(&self) -> u64;
    pub fn set_device_id(&mut self, device_id: u64);
    pub fn sync_metadata(&mut self) -> Result<()>;
}

struct InodeImpl {
    desc: Dirty<InodeDesc>,
    block_manager: Arc<InodeBlockManager>,
    is_freed: bool,
    last_alloc_device_bid: Option<Ext2Bid>,
    weak_self: Weak<Inode>,
}

impl InodeImpl {
    pub fn new(desc: Dirty<InodeDesc>, weak_self: Weak<Inode>, fs: Weak<Ext2>) -> Self {
        let block_manager = InodeBlockManager {
            nblocks: AtomicUsize::new(desc.blocks_count() as _),
            block_ptrs: RwMutex::new(desc.block_ptrs),
            indirect_blocks: RwMutex::new(IndirectBlockCache::new(fs.clone())),
            fs,
        };
        Self {
            desc,
            block_manager: Arc::new(block_manager),
            is_freed: false,
            last_alloc_device_bid: None,
            weak_self,
        }
    }

    pub fn file_size(&self) -> usize {
        self.desc.size
    }

    pub fn inode_type(&self) -> InodeType {
        self.desc.type_
    }

    pub fn inode(&self) -> Arc<Inode> {
        self.weak_self.upgrade().unwrap()
    }

    pub fn fs(&self) -> Arc<Ext2> {
        self.inode().fs()
    }

    pub fn file_perm(&self) -> FilePerm {
        self.desc.perm
    }

    pub fn set_file_perm(&mut self, perm: FilePerm) {
        self.desc.perm = perm;
    }

    pub fn uid(&self) -> u32 {
        self.desc.uid
    }

    pub fn set_uid(&mut self, uid: u32) {
        self.desc.uid = uid;
    }

    pub fn gid(&self) -> u32 {
        self.desc.gid
    }

    pub fn set_gid(&mut self, gid: u32) {
        self.desc.gid = gid;
    }

    pub fn file_flags(&self) -> FileFlags {
        self.desc.flags
    }

    pub fn hard_links(&self) -> u16 {
        self.desc.hard_links
    }

    pub fn inc_hard_links(&mut self) {
        self.desc.hard_links += 1;
    }

    pub fn dec_hard_links(&mut self) {
        debug_assert!(self.hard_links() > 0);
        self.desc.hard_links -= 1;
    }

    pub fn blocks_count(&self) -> Ext2Bid {
        self.desc.blocks_count()
    }

    pub fn acl(&self) -> Option<Bid> {
        self.desc.acl
    }

    pub fn set_acl(&mut self, bid: Bid) {
        self.desc.acl = Some(bid);
    }

    pub fn atime(&self) -> Duration {
        self.desc.atime
    }

    pub fn set_atime(&mut self, time: Duration) {
        self.desc.atime = time;
    }

    pub fn mtime(&self) -> Duration {
        self.desc.mtime
    }

    pub fn set_mtime(&mut self, time: Duration) {
        self.desc.mtime = time;
    }

    pub fn ctime(&self) -> Duration {
        self.desc.ctime
    }

    pub fn set_ctime(&mut self, time: Duration) {
        self.desc.ctime = time;
    }

    pub fn set_device_id(&mut self, device_id: u64) {
        self.desc.block_ptrs.as_bytes_mut()[..core::mem::size_of::<u64>()]
            .copy_from_slice(device_id.as_bytes());
        self.block_manager.block_ptrs.write().as_bytes_mut()[..core::mem::size_of::<u64>()]
            .copy_from_slice(device_id.as_bytes());
    }

    pub fn device_id(&self) -> u64 {
        let mut device_id: u64 = 0;
        device_id
            .as_bytes_mut()
            .copy_from_slice(&self.desc.block_ptrs.as_bytes()[..core::mem::size_of::<u64>()]);
        device_id
    }

    pub fn read_link(&self) -> Result<String> {
        let symlink_str = core::str::from_utf8(&self.desc.block_ptrs.as_bytes()[..self.desc.size])?;
        Ok(symlink_str.to_owned())
    }

    pub fn write_link(&mut self, target: &str) -> Result<()> {
        let target_len = target.len();
        self.desc.block_ptrs.as_bytes_mut()[..target_len].copy_from_slice(target.as_bytes());
        self.block_manager.block_ptrs.write().as_bytes_mut()[..target_len]
            .copy_from_slice(target.as_bytes());
        if self.desc.size != target_len {
            self.resize(target_len)?;
        }
        Ok(())
    }

    pub fn sync_metadata(&mut self) -> Result<()> {
        if !self.desc.is_dirty() {
            return Ok(());
        }

        let inode = self.inode();
        if self.hard_links() == 0 {
            self.resize(0)?;
            // Adds the check here to prevent double-free.
            if !self.is_freed {
                inode
                    .fs()
                    .free_inode(inode.ino(), self.desc.type_ == InodeType::Dir)?;
                if let Some(xattr) = &inode.xattr {
                    xattr.free()?;
                }
                self.is_freed = true;
            }
        }

        self.block_manager.indirect_blocks.write().evict_all()?;
        inode.fs().sync_inode(inode.ino(), &self.desc)?;
        self.desc.clear_dirty();
        Ok(())
    }
}

// Heavy implementation for inode resizing.
impl InodeImpl {
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
        }

        // Expands the size
        self.update_size(new_size);
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
                let mut block_ptrs = self.block_manager.block_ptrs.write();
                for (i, bid) in device_range.enumerate() {
                    self.desc.block_ptrs.set_direct(idx as usize + i, bid);
                    block_ptrs.set_direct(idx as usize + i, bid);
                }
            }
            BidPath::Indirect(idx) => {
                let indirect_bid = self.desc.block_ptrs.indirect();
                assert!(indirect_bid != 0);
                let mut indirect_blocks = self.block_manager.indirect_blocks.write();
                let indirect_block = indirect_blocks.find_mut(indirect_bid)?;
                for (i, bid) in device_range.enumerate() {
                    indirect_block.write_bid(idx as usize + i, &bid)?;
                }
            }
            BidPath::DbIndirect(lvl1_idx, lvl2_idx) => {
                let mut indirect_blocks = self.block_manager.indirect_blocks.write();
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
                let mut indirect_blocks = self.block_manager.indirect_blocks.write();
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

        let bid_path = BidPath::from(bid);
        let mut block_ptrs = self.block_manager.block_ptrs.write();
        let mut indirect_blocks = self.block_manager.indirect_blocks.write();
        for indirect_bid in indirect_bids.iter() {
            let indirect_block = IndirectBlock::alloc()?;
            indirect_blocks.insert(*indirect_bid, indirect_block)?;

            match bid_path {
                BidPath::Indirect(idx) => {
                    assert_eq!(idx, 0);
                    self.desc.block_ptrs.set_indirect(*indirect_bid);
                    block_ptrs.set_indirect(*indirect_bid);
                }
                BidPath::DbIndirect(lvl1_idx, lvl2_idx) => {
                    assert_eq!(lvl2_idx, 0);
                    if self.desc.block_ptrs.db_indirect() == 0 {
                        self.desc.block_ptrs.set_db_indirect(*indirect_bid);
                        block_ptrs.set_db_indirect(*indirect_bid);
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
                        block_ptrs.set_tb_indirect(*indirect_bid);
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
    /// After the reduction, the size will be shrunk to `new_size`,
    /// which may result in an decreased block count.
    fn shrink(&mut self, new_size: usize) {
        let new_blocks = self.desc.size_to_blocks(new_size);
        let old_blocks = self.desc.blocks_count();

        // Shrinks block count if necessary
        if new_blocks < old_blocks {
            self.shrink_blocks(new_blocks..old_blocks);
        }

        // Shrinks the size
        self.update_size(new_size);
    }

    fn update_size(&mut self, new_size: usize) {
        self.desc.size = new_size;
        self.block_manager
            .nblocks
            .store(self.blocks_count() as _, Ordering::Release);
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
                DeviceRangeReader::new(&self.block_manager, (range.start - 1)..range.start)
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
        let device_range_reader =
            DeviceRangeReader::new(&self.block_manager, range.clone()).unwrap();
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

        let mut block_ptrs = self.block_manager.block_ptrs.write();
        let mut indirect_blocks = self.block_manager.indirect_blocks.write();
        match bid_path {
            BidPath::Indirect(_) => {
                let indirect_bid = self.desc.block_ptrs.indirect();
                if indirect_bid == 0 {
                    return Ok(());
                }

                self.desc.block_ptrs.set_indirect(bid);
                block_ptrs.set_indirect(bid);
                indirect_blocks.remove(indirect_bid);
                self.fs()
                    .free_blocks(indirect_bid..indirect_bid + 1)
                    .unwrap();
            }
            BidPath::DbIndirect(lvl1_idx, _) => {
                let db_indirect_bid = self.desc.block_ptrs.db_indirect();
                if db_indirect_bid == 0 {
                    return Ok(());
                }

                let fs = self.fs();
                let lvl1_indirect_bid = {
                    let db_indirect_block = indirect_blocks.find(db_indirect_bid)?;
                    db_indirect_block.read_bid(lvl1_idx as usize)?
                };
                if lvl1_indirect_bid != 0 {
                    indirect_blocks.remove(lvl1_indirect_bid);
                    fs.free_blocks(lvl1_indirect_bid..lvl1_indirect_bid + 1)
                        .unwrap();
                }
                if lvl1_idx == 0 {
                    self.desc.block_ptrs.set_db_indirect(0);
                    block_ptrs.set_db_indirect(0);
                    indirect_blocks.remove(db_indirect_bid);
                    fs.free_blocks(db_indirect_bid..db_indirect_bid + 1)
                        .unwrap();
                }
            }
            BidPath::TbIndirect(lvl1_idx, lvl2_idx, _) => {
                let tb_indirect_bid = self.desc.block_ptrs.tb_indirect();
                if tb_indirect_bid == 0 {
                    return Ok(());
                }

                let fs = self.fs();
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
                        fs.free_blocks(lvl2_indirect_bid..lvl2_indirect_bid + 1)
                            .unwrap();
                    }
                    if lvl2_idx == 0 {
                        indirect_blocks.remove(lvl1_indirect_bid);
                        fs.free_blocks(lvl1_indirect_bid..lvl1_indirect_bid + 1)
                            .unwrap();
                    }
                }

                if lvl2_idx == 0 && lvl1_idx == 0 {
                    self.desc.block_ptrs.set_tb_indirect(0);
                    block_ptrs.set_tb_indirect(0);
                    indirect_blocks.remove(tb_indirect_bid);
                    fs.free_blocks(tb_indirect_bid..tb_indirect_bid + 1)
                        .unwrap();
                }
            }
            BidPath::Direct(_) => panic!(),
        }

        Ok(())
    }
}

#[inherit_methods(from = "self.block_manager")]
impl InodeImpl {
    pub fn read_blocks_async(
        &self,
        bid: Ext2Bid,
        nblocks: usize,
        writer: &mut VmWriter,
    ) -> Result<BioWaiter>;
    pub fn read_blocks(&self, bid: Ext2Bid, nblocks: usize, writer: &mut VmWriter) -> Result<()>;
    pub fn read_block_async(&self, bid: Ext2Bid, frame: &CachePage) -> Result<BioWaiter>;
    pub fn write_blocks_async(
        &self,
        bid: Ext2Bid,
        nblocks: usize,
        reader: &mut VmReader,
    ) -> Result<BioWaiter>;
    pub fn write_blocks(&self, bid: Ext2Bid, nblocks: usize, reader: &mut VmReader) -> Result<()>;
    pub fn write_block_async(&self, bid: Ext2Bid, frame: &CachePage) -> Result<BioWaiter>;
}

/// Manages the inode blocks and block I/O operations.
struct InodeBlockManager {
    nblocks: AtomicUsize,
    /// Maintains a second copy of block pointers for page cache use, distinct from
    /// the one in `InodeDesc`.
    ///
    /// Updates occur infrequently and are performed on both copies, whereas
    /// frequent reads access the `InodeDesc` copy without locking.
    block_ptrs: RwMutex<BlockPtrs>,
    indirect_blocks: RwMutex<IndirectBlockCache>,
    fs: Weak<Ext2>,
}

impl InodeBlockManager {
    /// Reads one or multiple blocks to the segment start from `bid` asynchronously.
    pub fn read_blocks_async(
        &self,
        bid: Ext2Bid,
        nblocks: usize,
        writer: &mut VmWriter,
    ) -> Result<BioWaiter> {
        debug_assert!(nblocks * BLOCK_SIZE <= writer.avail());
        let mut bio_waiter = BioWaiter::new();

        for dev_range in DeviceRangeReader::new(self, bid..bid + nblocks as Ext2Bid)? {
            let start_bid = dev_range.start as Ext2Bid;
            let range_nblocks = dev_range.len();

            let bio_segment = BioSegment::alloc(range_nblocks, BioDirection::FromDevice);
            bio_segment.reader().unwrap().read_fallible(writer)?;

            let waiter = self.fs().read_blocks_async(start_bid, bio_segment)?;
            bio_waiter.concat(waiter);
        }

        Ok(bio_waiter)
    }

    pub fn read_blocks(&self, bid: Ext2Bid, nblocks: usize, writer: &mut VmWriter) -> Result<()> {
        match self.read_blocks_async(bid, nblocks, writer)?.wait() {
            Some(BioStatus::Complete) => Ok(()),
            _ => return_errno!(Errno::EIO),
        }
    }

    pub fn read_block_async(&self, bid: Ext2Bid, frame: &CachePage) -> Result<BioWaiter> {
        let mut bio_waiter = BioWaiter::new();

        for dev_range in DeviceRangeReader::new(self, bid..bid + 1 as Ext2Bid)? {
            let start_bid = dev_range.start as Ext2Bid;
            // TODO: Should we allocate the bio segment from the pool on reads?
            // This may require an additional copy to the requested frame in the completion callback.
            let bio_segment = BioSegment::new_from_segment(
                Segment::from(frame.clone()).into(),
                BioDirection::FromDevice,
            );
            let waiter = self.fs().read_blocks_async(start_bid, bio_segment)?;
            bio_waiter.concat(waiter);
        }

        Ok(bio_waiter)
    }

    /// Writes one or multiple blocks from the segment start from `bid` asynchronously.
    pub fn write_blocks_async(
        &self,
        bid: Ext2Bid,
        nblocks: usize,
        reader: &mut VmReader,
    ) -> Result<BioWaiter> {
        debug_assert_eq!(nblocks * BLOCK_SIZE, reader.remain());
        let mut bio_waiter = BioWaiter::new();

        for dev_range in DeviceRangeReader::new(self, bid..bid + nblocks as Ext2Bid)? {
            let start_bid = dev_range.start as Ext2Bid;
            let range_nblocks = dev_range.len();

            let bio_segment = BioSegment::alloc(range_nblocks, BioDirection::ToDevice);
            bio_segment.writer().unwrap().write_fallible(reader)?;

            let waiter = self.fs().write_blocks_async(start_bid, bio_segment)?;
            bio_waiter.concat(waiter);
        }

        Ok(bio_waiter)
    }

    pub fn write_blocks(&self, bid: Ext2Bid, nblocks: usize, reader: &mut VmReader) -> Result<()> {
        match self.write_blocks_async(bid, nblocks, reader)?.wait() {
            Some(BioStatus::Complete) => Ok(()),
            _ => return_errno!(Errno::EIO),
        }
    }

    pub fn write_block_async(&self, bid: Ext2Bid, frame: &CachePage) -> Result<BioWaiter> {
        let mut bio_waiter = BioWaiter::new();

        for dev_range in DeviceRangeReader::new(self, bid..bid + 1 as Ext2Bid)? {
            let start_bid = dev_range.start as Ext2Bid;
            let bio_segment = BioSegment::alloc(1, BioDirection::ToDevice);
            // This requires an additional copy to the pooled bio segment.
            bio_segment
                .writer()
                .unwrap()
                .write_fallible(&mut frame.reader().to_fallible())?;
            let waiter = self.fs().write_blocks_async(start_bid, bio_segment)?;
            bio_waiter.concat(waiter);
        }

        Ok(bio_waiter)
    }

    pub fn nblocks(&self) -> usize {
        self.nblocks.load(Ordering::Acquire)
    }

    pub fn fs(&self) -> Arc<Ext2> {
        self.fs.upgrade().unwrap()
    }
}

impl PageCacheBackend for InodeBlockManager {
    fn read_page_async(&self, idx: usize, frame: &CachePage) -> Result<BioWaiter> {
        let bid = idx as Ext2Bid;
        self.read_block_async(bid, frame)
    }

    fn write_page_async(&self, idx: usize, frame: &CachePage) -> Result<BioWaiter> {
        let bid = idx as Ext2Bid;
        self.write_block_async(bid, frame)
    }

    fn npages(&self) -> usize {
        self.nblocks()
    }
}

/// A reader to get the corresponding device block IDs for a specified range.
///
/// It calculates and returns the range of block IDs on the device that would map to
/// the file's block range. This is useful for translating file-level block addresses
/// to their locations on the physical storage device.
struct DeviceRangeReader<'a> {
    block_ptrs: RwMutexReadGuard<'a, BlockPtrs>,
    indirect_blocks: RwMutexWriteGuard<'a, IndirectBlockCache>,
    range: Range<Ext2Bid>,
    indirect_block: Option<IndirectBlock>,
}

impl<'a> DeviceRangeReader<'a> {
    /// Creates a new reader.
    ///
    /// # Panics
    ///
    /// If the 'range' is empty, this method will panic.
    pub fn new(block_manager: &'a InodeBlockManager, range: Range<Ext2Bid>) -> Result<Self> {
        assert!(!range.is_empty());

        let mut reader = Self {
            block_ptrs: block_manager.block_ptrs.read(),
            indirect_blocks: block_manager.indirect_blocks.write(),
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
                None => self.block_ptrs.direct(i),
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
                let indirect_bid = self.block_ptrs.indirect();
                let indirect_block = self.indirect_blocks.find(indirect_bid)?;
                self.indirect_block = Some(indirect_block.clone());
            }
            BidPath::DbIndirect(lvl1_idx, _) => {
                let lvl1_indirect_bid = {
                    let db_indirect_block =
                        self.indirect_blocks.find(self.block_ptrs.db_indirect())?;
                    db_indirect_block.read_bid(lvl1_idx as usize)?
                };
                let lvl1_indirect_block = self.indirect_blocks.find(lvl1_indirect_bid)?;
                self.indirect_block = Some(lvl1_indirect_block.clone())
            }
            BidPath::TbIndirect(lvl1_idx, lvl2_idx, _) => {
                let lvl2_indirect_bid = {
                    let lvl1_indirect_bid = {
                        let tb_indirect_block =
                            self.indirect_blocks.find(self.block_ptrs.tb_indirect())?;
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

impl Iterator for DeviceRangeReader<'_> {
    type Item = Range<Ext2Bid>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.range.is_empty() {
            return None;
        }

        let range = self.read().unwrap();
        Some(range)
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
    type_: InodeType,
    /// Permission.
    perm: FilePerm,
    /// User Id.
    uid: u32,
    /// Group Id.
    gid: u32,
    /// Size in bytes.
    size: usize,
    /// Access time. This is the time when the file was last accessed.
    atime: Duration,
    /// Change time. This timestamp gets updated when the file's metadata changes.
    ctime: Duration,
    /// Modification time. This timestamp records the last modification of the file's content.
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
            dtime: Duration::from(inode.dtime),
            hard_links: inode.hard_links,
            blocks_count: inode.blocks_count,
            flags: FileFlags::from_bits(inode.flags)
                .ok_or(Error::with_message(Errno::EINVAL, "invalid file flags"))?,
            block_ptrs: inode.block_ptrs,
            acl: match inode_type {
                InodeType::File => Some(Bid::new(inode.file_acl as _)),
                InodeType::Dir => Some(Bid::new(inode.size_high as _)),
                _ => None,
            },
        })
    }
}

impl InodeDesc {
    pub fn new(type_: InodeType, perm: FilePerm) -> Dirty<Self> {
        let now = now();
        let credentials = current_thread!().as_posix_thread().unwrap().credentials();
        Dirty::new_dirty(Self {
            type_,
            perm,
            uid: credentials.fsuid().into(),
            gid: credentials.fsgid().into(),
            size: 0,
            atime: now,
            ctime: now,
            mtime: now,
            dtime: Duration::ZERO,
            hard_links: 1,
            blocks_count: 0,
            flags: FileFlags::empty(),
            block_ptrs: BlockPtrs::default(),
            acl: match type_ {
                InodeType::File | InodeType::Dir => Some(Bid::new(0)),
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

    fn size_to_blocks(&self, size: usize) -> Ext2Bid {
        if self.type_ == InodeType::SymLink && size <= MAX_FAST_SYMLINK_LEN {
            return 0;
        }
        size.div_ceil(BLOCK_SIZE) as Ext2Bid
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
    /// Change time.
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
                Some(acl) if inode.type_ == InodeType::File => acl.to_raw() as u32,
                _ => Default::default(),
            },
            size_high: match inode.acl {
                Some(acl) if inode.type_ == InodeType::Dir => acl.to_raw() as u32,
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
