use alloc::str;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;
use jinux_frame::sync::{RwLock, RwLockWriteGuard};
use jinux_frame::vm::VmFrame;
use jinux_frame::GenericIo;
use jinux_rights::Full;
use jinux_util::slot_vec::SlotVec;

use super::*;
use crate::fs::device::Device;
use crate::fs::utils::{
    DirentVisitor, FileSystem, FsFlags, Inode, InodeMode, InodeType, IoEvents, IoctlCmd, Metadata,
    PageCache, Poller, SuperBlock, NAME_MAX,
};
use crate::prelude::*;
use crate::vm::vmo::Vmo;

/// A volatile file system whose data and metadata exists only in memory.
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

    pub fn new_file(
        ino: usize,
        mode: InodeMode,
        sb: &SuperBlock,
        weak_inode: Weak<RamInode>,
    ) -> Self {
        Self {
            inner: Inner::File(PageCache::new(0, weak_inode).unwrap()),
            metadata: Metadata::new_file(ino, mode, sb),
            this: Weak::default(),
            fs: Weak::default(),
        }
    }

    pub fn new_symlink(ino: usize, mode: InodeMode, sb: &SuperBlock) -> Self {
        Self {
            inner: Inner::SymLink(String::from("")),
            metadata: Metadata::new_symlink(ino, mode, sb),
            this: Weak::default(),
            fs: Weak::default(),
        }
    }

    pub fn new_socket(ino: usize, mode: InodeMode, sb: &SuperBlock) -> Self {
        Self {
            inner: Inner::Socket,
            metadata: Metadata::new_socket(ino, mode, sb),
            this: Weak::default(),
            fs: Weak::default(),
        }
    }

    pub fn new_device(
        ino: usize,
        mode: InodeMode,
        sb: &SuperBlock,
        device: Arc<dyn Device>,
    ) -> Self {
        Self {
            metadata: Metadata::new_device(ino, mode, sb, device.as_ref()),
            inner: Inner::Device(device),
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

#[allow(clippy::large_enum_variant)]
enum Inner {
    Dir(DirEntry),
    File(PageCache),
    SymLink(String),
    Device(Arc<dyn Device>),
    Socket,
}

impl Inner {
    fn as_file(&self) -> Option<&PageCache> {
        match self {
            Inner::File(page_cache) => Some(page_cache),
            _ => None,
        }
    }

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

    fn as_symlink_mut(&mut self) -> Option<&mut String> {
        match self {
            Inner::SymLink(link) => Some(link),
            _ => None,
        }
    }

    fn as_device(&self) -> Option<&Arc<dyn Device>> {
        match self {
            Inner::Device(device) => Some(device),
            _ => None,
        }
    }
}

struct DirEntry {
    children: SlotVec<(String, Arc<RamInode>)>,
    this: Weak<RamInode>,
    parent: Weak<RamInode>,
}

impl DirEntry {
    fn new() -> Self {
        Self {
            children: SlotVec::new(),
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
            self.children.iter().any(|(child, _)| child == name)
        }
    }

    fn get_entry(&self, name: &str) -> Option<(usize, Arc<RamInode>)> {
        if name == "." {
            Some((0, self.this.upgrade().unwrap()))
        } else if name == ".." {
            Some((1, self.parent.upgrade().unwrap()))
        } else {
            self.children
                .idxes_and_items()
                .find(|(_, (child, _))| child == name)
                .map(|(idx, (_, inode))| (idx + 2, inode.clone()))
        }
    }

    fn append_entry(&mut self, name: &str, inode: Arc<RamInode>) -> usize {
        self.children.put((String::from(name), inode))
    }

    fn remove_entry(&mut self, idx: usize) -> Option<(String, Arc<RamInode>)> {
        assert!(idx >= 2);
        self.children.remove(idx - 2)
    }

    fn substitute_entry(
        &mut self,
        idx: usize,
        new_entry: (String, Arc<RamInode>),
    ) -> Option<(String, Arc<RamInode>)> {
        assert!(idx >= 2);
        self.children.put_at(idx - 2, new_entry)
    }

    fn visit_entry(&self, idx: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
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
            let start_idx = *idx;
            for (offset, (name, child)) in self
                .children
                .idxes_and_items()
                .map(|(offset, (name, child))| (offset + 2, (name, child)))
                .skip_while(|(offset, _)| offset < &start_idx)
            {
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

        let mut iterate_idx = idx;
        match try_visit(&mut iterate_idx, visitor) {
            Err(e) if idx == iterate_idx => Err(e),
            _ => Ok(iterate_idx - idx),
        }
    }

    fn is_empty_children(&self) -> bool {
        self.children.is_empty()
    }
}

impl RamInode {
    fn new_dir(fs: &Arc<RamFS>, mode: InodeMode, parent: &Weak<Self>) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| {
            let inode = RamInode(RwLock::new(Inode_::new_dir(fs.alloc_id(), mode, &fs.sb())));
            inode.0.write().fs = Arc::downgrade(fs);
            inode.0.write().this = weak_self.clone();
            inode
                .0
                .write()
                .inner
                .as_direntry_mut()
                .unwrap()
                .init(weak_self.clone(), parent.clone());
            inode
        })
    }

    fn new_file(fs: &Arc<RamFS>, mode: InodeMode) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| {
            let inode = RamInode(RwLock::new(Inode_::new_file(
                fs.alloc_id(),
                mode,
                &fs.sb(),
                weak_self.clone(),
            )));
            inode.0.write().fs = Arc::downgrade(fs);
            inode.0.write().this = weak_self.clone();
            inode
        })
    }

    fn new_socket(fs: &Arc<RamFS>, mode: InodeMode) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| {
            let inode = RamInode(RwLock::new(Inode_::new_socket(
                fs.alloc_id(),
                mode,
                &fs.sb(),
            )));
            inode.0.write().fs = Arc::downgrade(fs);
            inode.0.write().this = weak_self.clone();
            inode
        })
    }

    fn new_symlink(fs: &Arc<RamFS>, mode: InodeMode) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| {
            let inode = RamInode(RwLock::new(Inode_::new_symlink(
                fs.alloc_id(),
                mode,
                &fs.sb(),
            )));
            inode.0.write().fs = Arc::downgrade(fs);
            inode.0.write().this = weak_self.clone();
            inode
        })
    }

    fn new_device(fs: &Arc<RamFS>, mode: InodeMode, device: Arc<dyn Device>) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| {
            let inode = RamInode(RwLock::new(Inode_::new_device(
                fs.alloc_id(),
                mode,
                &fs.sb(),
                device,
            )));
            inode.0.write().fs = Arc::downgrade(fs);
            inode.0.write().this = weak_self.clone();
            inode
        })
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

    fn page_cache(&self) -> Option<Vmo<Full>> {
        self.0
            .read()
            .inner
            .as_file()
            .map(|page_cache| page_cache.pages().dup().unwrap())
    }

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        if let Some(device) = self.0.read().inner.as_device() {
            return device.read(buf);
        }

        let self_inode = self.0.read();
        if let Some(page_cache) = self_inode.inner.as_file() {
            let (offset, read_len) = {
                let file_len = self_inode.metadata.size;
                let start = file_len.min(offset);
                let end = file_len.min(offset + buf.len());
                (start, end - start)
            };
            page_cache
                .pages()
                .read_bytes(offset, &mut buf[..read_len])?;
            return Ok(read_len);
        }

        return_errno_with_message!(Errno::EINVAL, "direct read is not supported");
    }

    fn read_direct_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        self.read_at(offset, buf)
    }

    fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        if let Some(device) = self.0.read().inner.as_device() {
            return device.write(buf);
        }

        let self_inode = self.0.read();
        if let Some(page_cache) = self_inode.inner.as_file() {
            let file_len = self_inode.metadata.size;
            let new_len = offset + buf.len();
            let should_expand_len = new_len > file_len;
            if should_expand_len {
                page_cache.pages().resize(new_len)?;
            }
            page_cache.pages().write_bytes(offset, buf)?;
            if should_expand_len {
                drop(self_inode);
                self.resize(new_len);
            }
            return Ok(buf.len());
        }

        return_errno_with_message!(Errno::EINVAL, "write is not supported");
    }

    fn write_direct_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        self.write_at(offset, buf)
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

    fn set_mode(&self, mode: InodeMode) {
        self.0.write().metadata.mode = mode;
    }

    fn mknod(
        &self,
        name: &str,
        mode: InodeMode,
        device: Arc<dyn Device>,
    ) -> Result<Arc<dyn Inode>> {
        if self.0.read().metadata.type_ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "self is not dir");
        }
        let mut self_inode = self.0.write();
        if self_inode.inner.as_direntry().unwrap().contains_entry(name) {
            return_errno_with_message!(Errno::EEXIST, "entry exists");
        }
        let device_inode = RamInode::new_device(&self_inode.fs.upgrade().unwrap(), mode, device);
        self_inode
            .inner
            .as_direntry_mut()
            .unwrap()
            .append_entry(name, device_inode.clone());
        self_inode.inc_size();
        Ok(device_inode)
    }

    fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<dyn Inode>> {
        if self.0.read().metadata.type_ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "self is not dir");
        }
        let mut self_inode = self.0.write();
        if self_inode.inner.as_direntry().unwrap().contains_entry(name) {
            return_errno_with_message!(Errno::EEXIST, "entry exists");
        }
        let fs = self_inode.fs.upgrade().unwrap();
        let new_inode = match type_ {
            InodeType::File => RamInode::new_file(&fs, mode),
            InodeType::SymLink => RamInode::new_symlink(&fs, mode),
            InodeType::Socket => RamInode::new_socket(&fs, mode),
            InodeType::Dir => {
                let dir_inode = RamInode::new_dir(&fs, mode, &self_inode.this);
                self_inode.metadata.nlinks += 1;
                dir_inode
            }
            _ => {
                panic!("unsupported inode type");
            }
        };
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
            self_dir.substitute_entry(idx, (String::from(new_name), inode));
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
        *link = String::from(target);
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

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        if let Some(device) = self.0.read().inner.as_device() {
            device.poll(mask, poller)
        } else {
            let events = IoEvents::IN | IoEvents::OUT;
            events & mask
        }
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        Weak::upgrade(&self.0.read().fs).unwrap()
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        if let Some(device) = self.0.read().inner.as_device() {
            return device.ioctl(cmd, arg);
        }
        return_errno_with_message!(Errno::EINVAL, "ioctl is not supported");
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
