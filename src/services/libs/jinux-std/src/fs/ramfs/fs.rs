use crate::prelude::*;
use alloc::str;
use alloc::string::String;
use core::any::Any;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;
use jinux_frame::vm::VmFrame;
use spin::{RwLock, RwLockWriteGuard};

use super::*;
use crate::fs::utils::{
    DirEntryVec, DirentVisitor, FileSystem, FsFlags, Inode, InodeMode, InodeType, IoctlCmd,
    Metadata, SuperBlock,
};

pub struct RamFS {
    metadata: RwLock<SuperBlock>,
    root: Arc<RamInode>,
    inode_allocator: AtomicUsize,
}

impl RamFS {
    pub fn new() -> Arc<Self> {
        let sb = SuperBlock::new(RAMFS_MAGIC, BLOCK_SIZE, NAME_MAX);
        let root = Arc::new(RamInode(RwLock::new(Inode_::new_dir(
            ROOT_INO,
            InodeMode::from_bits_truncate(0o755),
            &sb,
        ))));
        let ramfs = Arc::new(Self {
            metadata: RwLock::new(sb),
            root,
            inode_allocator: AtomicUsize::new(ROOT_INO + 1),
        });
        let mut root = ramfs.root.0.write();
        root.inner
            .as_direntry_mut()
            .unwrap()
            .init(Arc::downgrade(&ramfs.root), Arc::downgrade(&ramfs.root));
        root.this = Arc::downgrade(&ramfs.root);
        root.fs = Arc::downgrade(&ramfs);
        drop(root);
        ramfs
    }

    fn alloc_id(&self) -> usize {
        let next_id = self.inode_allocator.fetch_add(1, Ordering::SeqCst);
        self.metadata.write().files += 1;
        next_id
    }
}

impl FileSystem for RamFS {
    fn sync(&self) -> Result<()> {
        // do nothing
        Ok(())
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }

    fn sb(&self) -> SuperBlock {
        self.metadata.read().clone()
    }

    fn flags(&self) -> FsFlags {
        FsFlags::DENTRY_UNEVICTABLE
    }

    fn as_any_ref(&self) -> &dyn Any {
        self
    }
}

struct RamInode(RwLock<Inode_>);

struct Inode_ {
    inner: Inner,
    metadata: Metadata,
    this: Weak<RamInode>,
    fs: Weak<RamFS>,
}

impl Inode_ {
    pub fn new_dir(ino: usize, mode: InodeMode, sb: &SuperBlock) -> Self {
        Self {
            inner: Inner::Dir(DirEntry::new()),
            metadata: Metadata::new_dir(ino, mode, sb),
            this: Weak::default(),
            fs: Weak::default(),
        }
    }

    pub fn new_file(ino: usize, mode: InodeMode, sb: &SuperBlock) -> Self {
        Self {
            inner: Inner::File,
            metadata: Metadata::new_file(ino, mode, sb),
            this: Weak::default(),
            fs: Weak::default(),
        }
    }

    pub fn new_symlink(ino: usize, mode: InodeMode, sb: &SuperBlock) -> Self {
        Self {
            inner: Inner::SymLink(Str256::from("")),
            metadata: Metadata::new_symlink(ino, mode, sb),
            this: Weak::default(),
            fs: Weak::default(),
        }
    }

    pub fn inc_size(&mut self) {
        self.metadata.size += 1;
        self.metadata.blocks = (self.metadata.size + BLOCK_SIZE - 1) / BLOCK_SIZE;
    }

    pub fn dec_size(&mut self) {
        debug_assert!(self.metadata.size > 0);
        self.metadata.size -= 1;
        self.metadata.blocks = (self.metadata.size + BLOCK_SIZE - 1) / BLOCK_SIZE;
    }

    pub fn resize(&mut self, new_size: usize) {
        self.metadata.size = new_size;
        self.metadata.blocks = (new_size + BLOCK_SIZE - 1) / BLOCK_SIZE;
    }
}

enum Inner {
    Dir(DirEntry),
    File,
    SymLink(Str256),
}

impl Inner {
    fn as_direntry(&self) -> Option<&DirEntry> {
        match self {
            Inner::Dir(dir_entry) => Some(dir_entry),
            _ => None,
        }
    }

    fn as_direntry_mut(&mut self) -> Option<&mut DirEntry> {
        match self {
            Inner::Dir(dir_entry) => Some(dir_entry),
            _ => None,
        }
    }

    fn as_symlink(&self) -> Option<&str> {
        match self {
            Inner::SymLink(link) => Some(link.as_ref()),
            _ => None,
        }
    }

    fn as_symlink_mut(&mut self) -> Option<&mut Str256> {
        match self {
            Inner::SymLink(link) => Some(link),
            _ => None,
        }
    }
}

struct DirEntry {
    children: DirEntryVec<(Str256, Arc<RamInode>)>,
    this: Weak<RamInode>,
    parent: Weak<RamInode>,
}

impl DirEntry {
    fn new() -> Self {
        Self {
            children: DirEntryVec::new(),
            this: Weak::default(),
            parent: Weak::default(),
        }
    }

    fn init(&mut self, this: Weak<RamInode>, parent: Weak<RamInode>) {
        self.this = this;
        self.set_parent(parent);
    }

    fn set_parent(&mut self, parent: Weak<RamInode>) {
        self.parent = parent;
    }

    fn contains_entry(&self, name: &str) -> bool {
        if name == "." || name == ".." {
            true
        } else {
            self.children
                .iter()
                .find(|(child, _)| child == &Str256::from(name))
                .is_some()
        }
    }

    fn get_entry(&self, name: &str) -> Option<(usize, Arc<RamInode>)> {
        if name == "." {
            Some((0, self.this.upgrade().unwrap()))
        } else if name == ".." {
            Some((1, self.parent.upgrade().unwrap()))
        } else {
            self.children
                .idxes_and_entries()
                .find(|(_, (child, _))| child == &Str256::from(name))
                .map(|(idx, (_, inode))| (idx + 2, inode.clone()))
        }
    }

    fn append_entry(&mut self, name: &str, inode: Arc<RamInode>) {
        self.children.put((Str256::from(name), inode))
    }

    fn remove_entry(&mut self, idx: usize) -> Option<(Str256, Arc<RamInode>)> {
        assert!(idx >= 2);
        self.children.remove(idx - 2)
    }

    fn substitute_entry(
        &mut self,
        idx: usize,
        new_entry: (Str256, Arc<RamInode>),
    ) -> Option<(Str256, Arc<RamInode>)> {
        assert!(idx >= 2);
        self.children.put_at(idx - 2, new_entry)
    }

    fn visit_entry(&self, mut idx: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        let try_visit = |idx: &mut usize, visitor: &mut dyn DirentVisitor| -> Result<()> {
            // Read the two special entries("." and "..").
            if *idx == 0 {
                let this_inode = self.this.upgrade().unwrap();
                visitor.visit(
                    ".",
                    this_inode.metadata().ino as u64,
                    this_inode.metadata().type_,
                    *idx,
                )?;
                *idx += 1;
            }
            if *idx == 1 {
                let parent_inode = self.parent.upgrade().unwrap();
                visitor.visit(
                    "..",
                    parent_inode.metadata().ino as u64,
                    parent_inode.metadata().type_,
                    *idx,
                )?;
                *idx += 1;
            }
            // Read the normal child entries.
            for (offset, (name, child)) in self
                .children
                .idxes_and_entries()
                .map(|(offset, (name, child))| (offset + 2, (name, child)))
            {
                if offset < *idx {
                    continue;
                }
                visitor.visit(
                    name.as_ref(),
                    child.metadata().ino as u64,
                    child.metadata().type_,
                    offset,
                )?;
                *idx = offset + 1;
            }
            Ok(())
        };

        let initial_idx = idx;
        match try_visit(&mut idx, visitor) {
            Err(e) if idx == initial_idx => Err(e),
            _ => Ok(idx - initial_idx),
        }
    }

    fn is_empty_children(&self) -> bool {
        self.children.is_empty()
    }
}

#[repr(C)]
#[derive(Clone, PartialEq, PartialOrd, Eq, Ord)]
pub struct Str256([u8; 256]);

impl AsRef<str> for Str256 {
    fn as_ref(&self) -> &str {
        let len = self.0.iter().enumerate().find(|(_, &b)| b == 0).unwrap().0;
        str::from_utf8(&self.0[0..len]).unwrap()
    }
}

impl<'a> From<&'a str> for Str256 {
    fn from(s: &'a str) -> Self {
        let mut inner = [0u8; 256];
        let len = if s.len() > NAME_MAX {
            NAME_MAX
        } else {
            s.len()
        };
        inner[0..len].copy_from_slice(&s.as_bytes()[0..len]);
        Str256(inner)
    }
}

impl core::fmt::Debug for Str256 {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(f, "{}", self.as_ref())
    }
}

impl Inode for RamInode {
    fn read_page(&self, _idx: usize, _frame: &VmFrame) -> Result<()> {
        // do nothing
        Ok(())
    }

    fn write_page(&self, _idx: usize, _frame: &VmFrame) -> Result<()> {
        // do nothing
        Ok(())
    }

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "direct read is not supported");
    }

    fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "direct write is not supported");
    }

    fn len(&self) -> usize {
        self.0.read().metadata.size
    }

    fn resize(&self, new_size: usize) {
        self.0.write().resize(new_size)
    }

    fn atime(&self) -> Duration {
        self.0.read().metadata.atime
    }

    fn set_atime(&self, time: Duration) {
        self.0.write().metadata.atime = time;
    }

    fn mtime(&self) -> Duration {
        self.0.read().metadata.mtime
    }

    fn set_mtime(&self, time: Duration) {
        self.0.write().metadata.mtime = time;
    }

    fn mknod(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<dyn Inode>> {
        if self.0.read().metadata.type_ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "self is not dir");
        }
        let mut self_inode = self.0.write();
        if self_inode.inner.as_direntry().unwrap().contains_entry(name) {
            return_errno_with_message!(Errno::EEXIST, "entry exists");
        }
        let fs = self_inode.fs.upgrade().unwrap();
        let new_inode = match type_ {
            InodeType::File => {
                let file_inode = Arc::new(RamInode(RwLock::new(Inode_::new_file(
                    fs.alloc_id(),
                    mode,
                    &fs.sb(),
                ))));
                file_inode.0.write().fs = self_inode.fs.clone();
                file_inode
            }
            InodeType::Dir => {
                let dir_inode = Arc::new(RamInode(RwLock::new(Inode_::new_dir(
                    fs.alloc_id(),
                    mode,
                    &fs.sb(),
                ))));
                dir_inode.0.write().fs = self_inode.fs.clone();
                dir_inode.0.write().inner.as_direntry_mut().unwrap().init(
                    Arc::downgrade(&dir_inode),
                    self_inode.inner.as_direntry().unwrap().this.clone(),
                );
                self_inode.metadata.nlinks += 1;
                dir_inode
            }
            InodeType::SymLink => {
                let sym_inode = Arc::new(RamInode(RwLock::new(Inode_::new_symlink(
                    fs.alloc_id(),
                    mode,
                    &fs.sb(),
                ))));
                sym_inode.0.write().fs = self_inode.fs.clone();
                sym_inode
            }
            _ => {
                panic!("unsupported inode type");
            }
        };
        new_inode.0.write().this = Arc::downgrade(&new_inode);
        self_inode
            .inner
            .as_direntry_mut()
            .unwrap()
            .append_entry(name, new_inode.clone());
        self_inode.inc_size();
        Ok(new_inode)
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        if self.0.read().metadata.type_ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "self is not dir");
        }
        let self_inode = self.0.read();
        let cnt = self_inode
            .inner
            .as_direntry()
            .unwrap()
            .visit_entry(offset, visitor)?;
        Ok(cnt)
    }

    fn link(&self, old: &Arc<dyn Inode>, name: &str) -> Result<()> {
        if self.0.read().metadata.type_ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "self is not dir");
        }
        let old = old
            .downcast_ref::<RamInode>()
            .ok_or(Error::new(Errno::EXDEV))?;
        if old.0.read().metadata.type_ == InodeType::Dir {
            return_errno_with_message!(Errno::EPERM, "old is a dir");
        }
        let mut self_inode = self.0.write();
        if self_inode.inner.as_direntry().unwrap().contains_entry(name) {
            return_errno_with_message!(Errno::EEXIST, "entry exist");
        }

        self_inode
            .inner
            .as_direntry_mut()
            .unwrap()
            .append_entry(name, old.0.read().this.upgrade().unwrap());
        self_inode.inc_size();
        drop(self_inode);
        old.0.write().metadata.nlinks += 1;
        Ok(())
    }

    fn unlink(&self, name: &str) -> Result<()> {
        if self.0.read().metadata.type_ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "self is not dir");
        }
        if name == "." || name == ".." {
            return_errno_with_message!(Errno::EISDIR, "unlink . or ..");
        }
        let mut self_inode = self.0.write();
        let self_dir = self_inode.inner.as_direntry_mut().unwrap();
        let (idx, target) = self_dir.get_entry(name).ok_or(Error::new(Errno::ENOENT))?;
        if target.0.read().metadata.type_ == InodeType::Dir {
            return_errno_with_message!(Errno::EISDIR, "unlink on dir");
        }
        self_dir.remove_entry(idx);
        self_inode.dec_size();
        drop(self_inode);
        target.0.write().metadata.nlinks -= 1;
        Ok(())
    }

    fn rmdir(&self, name: &str) -> Result<()> {
        if self.0.read().metadata.type_ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "self is not dir");
        }
        if name == "." || name == ".." {
            return_errno_with_message!(Errno::EISDIR, "rmdir on . or ..");
        }
        let mut self_inode = self.0.write();
        let self_dir = self_inode.inner.as_direntry_mut().unwrap();
        let (idx, target) = self_dir.get_entry(name).ok_or(Error::new(Errno::ENOENT))?;
        if target.0.read().metadata.type_ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "rmdir on not dir");
        }
        if !target
            .0
            .read()
            .inner
            .as_direntry()
            .unwrap()
            .is_empty_children()
        {
            return_errno_with_message!(Errno::ENOTEMPTY, "dir not empty");
        }
        self_dir.remove_entry(idx);
        self_inode.dec_size();
        self_inode.metadata.nlinks -= 1;
        drop(self_inode);
        target.0.write().metadata.nlinks -= 2;
        Ok(())
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        if self.0.read().metadata.type_ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "self is not dir");
        }

        let (_, inode) = self
            .0
            .read()
            .inner
            .as_direntry()
            .unwrap()
            .get_entry(name)
            .ok_or(Error::new(Errno::ENOENT))?;
        Ok(inode as _)
    }

    fn rename(&self, old_name: &str, target: &Arc<dyn Inode>, new_name: &str) -> Result<()> {
        if self.0.read().metadata.type_ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "self is not dir");
        }
        let target = target
            .downcast_ref::<RamInode>()
            .ok_or(Error::new(Errno::EXDEV))?;
        if target.0.read().metadata.type_ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "target is not dir");
        }
        if old_name == "." || old_name == ".." {
            return_errno_with_message!(Errno::EISDIR, "old_name is . or ..");
        }
        if new_name == "." || new_name == ".." {
            return_errno_with_message!(Errno::EISDIR, "new_name is . or ..");
        }
        let src_inode = self.lookup(old_name)?;
        if src_inode.metadata().ino == target.metadata().ino {
            return_errno_with_message!(Errno::EINVAL, "target is a descendant of old");
        }
        if let Ok(dst_inode) = target.lookup(new_name) {
            if src_inode.metadata().ino == dst_inode.metadata().ino {
                return Ok(());
            }
            match (src_inode.metadata().type_, dst_inode.metadata().type_) {
                (InodeType::Dir, InodeType::Dir) => {
                    let dst_inode = dst_inode.downcast_ref::<RamInode>().unwrap();
                    if !dst_inode
                        .0
                        .read()
                        .inner
                        .as_direntry()
                        .unwrap()
                        .is_empty_children()
                    {
                        return_errno_with_message!(Errno::ENOTEMPTY, "dir not empty");
                    }
                }
                (InodeType::Dir, _) => {
                    return_errno_with_message!(Errno::ENOTDIR, "old is not dir");
                }
                (_, InodeType::Dir) => {
                    return_errno_with_message!(Errno::EISDIR, "new is dir");
                }
                _ => {}
            }
        }
        if self.metadata().ino == target.metadata().ino {
            let mut self_inode = self.0.write();
            let self_dir = self_inode.inner.as_direntry_mut().unwrap();
            let (idx, inode) = self_dir
                .get_entry(old_name)
                .ok_or(Error::new(Errno::ENOENT))?;
            self_dir.substitute_entry(idx, (Str256::from(new_name), inode));
        } else {
            let (mut self_inode, mut target_inode) = write_lock_two_inodes(self, target);
            let self_dir = self_inode.inner.as_direntry_mut().unwrap();
            let (idx, src_inode) = self_dir
                .get_entry(old_name)
                .ok_or(Error::new(Errno::ENOENT))?;
            self_dir.remove_entry(idx);
            target_inode
                .inner
                .as_direntry_mut()
                .unwrap()
                .append_entry(new_name, src_inode.clone());
            self_inode.dec_size();
            target_inode.inc_size();
            if src_inode.0.read().metadata.type_ == InodeType::Dir {
                self_inode.metadata.nlinks -= 1;
                target_inode.metadata.nlinks += 1;
            }
            drop(self_inode);
            drop(target_inode);
            if src_inode.0.read().metadata.type_ == InodeType::Dir {
                src_inode
                    .0
                    .write()
                    .inner
                    .as_direntry_mut()
                    .unwrap()
                    .set_parent(target.0.read().this.clone());
            }
        }
        Ok(())
    }

    fn read_link(&self) -> Result<String> {
        if self.0.read().metadata.type_ != InodeType::SymLink {
            return_errno_with_message!(Errno::EINVAL, "self is not symlink");
        }
        let self_inode = self.0.read();
        let link = self_inode.inner.as_symlink().unwrap();
        Ok(String::from(link))
    }

    fn write_link(&self, target: &str) -> Result<()> {
        if self.0.read().metadata.type_ != InodeType::SymLink {
            return_errno_with_message!(Errno::EINVAL, "self is not symlink");
        }
        let mut self_inode = self.0.write();
        let link = self_inode.inner.as_symlink_mut().unwrap();
        *link = Str256::from(target);
        // Symlink's metadata.blocks should be 0, so just set the size.
        self_inode.metadata.size = target.len();
        Ok(())
    }

    fn metadata(&self) -> Metadata {
        self.0.read().metadata.clone()
    }

    fn sync(&self) -> Result<()> {
        // do nothing
        Ok(())
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        Weak::upgrade(&self.0.read().fs).unwrap()
    }

    fn ioctl(&self, cmd: &IoctlCmd) -> Result<()> {
        return_errno!(Errno::ENOSYS);
    }

    fn as_any_ref(&self) -> &dyn Any {
        self
    }
}

fn write_lock_two_inodes<'a>(
    this: &'a RamInode,
    other: &'a RamInode,
) -> (RwLockWriteGuard<'a, Inode_>, RwLockWriteGuard<'a, Inode_>) {
    if this.0.read().metadata.ino < other.0.read().metadata.ino {
        let this = this.0.write();
        let other = other.0.write();
        (this, other)
    } else {
        let other = other.0.write();
        let this = this.0.write();
        (this, other)
    }
}
