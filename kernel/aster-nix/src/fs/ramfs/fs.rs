// SPDX-License-Identifier: MPL-2.0

use core::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use aster_block::bio::BioWaiter;
use aster_rights::Full;
use aster_util::slot_vec::SlotVec;
use ostd::{
    mm::{Frame, VmIo},
    sync::RwMutexWriteGuard,
};

use super::*;
use crate::{
    events::IoEvents,
    fs::{
        device::Device,
        utils::{
            CStr256, DirentVisitor, FileSystem, FsFlags, Inode, InodeMode, InodeType, IoctlCmd,
            Metadata, PageCache, PageCacheBackend, SuperBlock,
        },
    },
    prelude::*,
    process::{signal::Poller, Gid, Uid},
    time::clocks::RealTimeCoarseClock,
    vm::vmo::Vmo,
};

/// A volatile file system whose data and metadata exists only in memory.
pub struct RamFS {
    /// The super block
    sb: SuperBlock,
    /// Root inode
    root: Arc<RamInode>,
    /// An inode allocator
    inode_allocator: AtomicU64,
}

impl RamFS {
    pub fn new() -> Arc<Self> {
        Arc::new_cyclic(|weak_fs| Self {
            sb: SuperBlock::new(RAMFS_MAGIC, BLOCK_SIZE, NAME_MAX),
            root: Arc::new_cyclic(|weak_root| RamInode {
                node: RwMutex::new(Node::new_dir(
                    InodeMode::from_bits_truncate(0o755),
                    Uid::new_root(),
                    Gid::new_root(),
                    weak_root.clone(),
                    weak_root.clone(),
                )),
                ino: ROOT_INO,
                typ: InodeType::Dir,
                this: weak_root.clone(),
                fs: weak_fs.clone(),
            }),
            inode_allocator: AtomicU64::new(ROOT_INO + 1),
        })
    }

    fn alloc_id(&self) -> u64 {
        self.inode_allocator.fetch_add(1, Ordering::SeqCst)
    }

    fn device_id(&self) -> u64 {
        0
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
        self.sb.clone()
    }

    fn flags(&self) -> FsFlags {
        FsFlags::DENTRY_UNEVICTABLE
    }
}

struct RamInode {
    /// The mutable part of the inode
    node: RwMutex<Node>,
    /// Inode number
    ino: u64,
    /// Type of the inode
    typ: InodeType,
    /// Reference to self
    this: Weak<RamInode>,
    /// Reference to fs
    fs: Weak<RamFS>,
}

struct Node {
    inner: Inner,
    metadata: InodeMeta,
}

impl Node {
    pub fn new_dir(
        mode: InodeMode,
        uid: Uid,
        gid: Gid,
        this: Weak<RamInode>,
        parent: Weak<RamInode>,
    ) -> Self {
        Self {
            inner: Inner::Dir(DirEntry::new(this, parent)),
            metadata: InodeMeta::new_dir(mode, uid, gid),
        }
    }

    pub fn new_file(mode: InodeMode, uid: Uid, gid: Gid, this: Weak<RamInode>) -> Self {
        Self {
            inner: Inner::File(PageCache::new(this).unwrap()),
            metadata: InodeMeta::new(mode, uid, gid),
        }
    }

    pub fn new_symlink(mode: InodeMode, uid: Uid, gid: Gid) -> Self {
        Self {
            inner: Inner::SymLink(String::from("")),
            metadata: InodeMeta::new(mode, uid, gid),
        }
    }

    pub fn new_socket(mode: InodeMode, uid: Uid, gid: Gid) -> Self {
        Self {
            inner: Inner::Socket,
            metadata: InodeMeta::new(mode, uid, gid),
        }
    }

    pub fn new_device(mode: InodeMode, uid: Uid, gid: Gid, device: Arc<dyn Device>) -> Self {
        Self {
            inner: Inner::Device(device),
            metadata: InodeMeta::new(mode, uid, gid),
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

    pub fn atime(&self) -> Duration {
        self.metadata.atime
    }

    pub fn set_atime(&mut self, time: Duration) {
        self.metadata.atime = time;
    }

    pub fn mtime(&self) -> Duration {
        self.metadata.mtime
    }

    pub fn set_mtime(&mut self, time: Duration) {
        self.metadata.mtime = time;
    }

    pub fn ctime(&self) -> Duration {
        self.metadata.ctime
    }

    pub fn set_ctime(&mut self, time: Duration) {
        self.metadata.ctime = time;
    }

    pub fn inc_nlinks(&mut self) {
        self.metadata.nlinks += 1;
    }

    pub fn dec_nlinks(&mut self) {
        debug_assert!(self.metadata.nlinks > 0);
        self.metadata.nlinks -= 1;
    }
}

#[derive(Debug, Clone, Copy)]
struct InodeMeta {
    size: usize,
    blocks: usize,
    atime: Duration,
    mtime: Duration,
    ctime: Duration,
    mode: InodeMode,
    nlinks: usize,
    uid: Uid,
    gid: Gid,
}

impl InodeMeta {
    pub fn new(mode: InodeMode, uid: Uid, gid: Gid) -> Self {
        let now = now();
        Self {
            size: 0,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            mode,
            nlinks: 1,
            uid,
            gid,
        }
    }

    pub fn new_dir(mode: InodeMode, uid: Uid, gid: Gid) -> Self {
        let now = now();
        Self {
            size: 2,
            blocks: 1,
            atime: now,
            mtime: now,
            ctime: now,
            mode,
            nlinks: 2,
            uid,
            gid,
        }
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
    children: SlotVec<(CStr256, Arc<RamInode>)>,
    this: Weak<RamInode>,
    parent: Weak<RamInode>,
}

impl DirEntry {
    fn new(this: Weak<RamInode>, parent: Weak<RamInode>) -> Self {
        Self {
            children: SlotVec::new(),
            this,
            parent,
        }
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
                .any(|(child, _)| child.as_str().unwrap() == name)
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
                .find(|(_, (child, _))| child.as_str().unwrap() == name)
                .map(|(idx, (_, inode))| (idx + 2, inode.clone()))
        }
    }

    fn append_entry(&mut self, name: &str, inode: Arc<RamInode>) -> usize {
        self.children.put((CStr256::from(name), inode))
    }

    fn remove_entry(&mut self, idx: usize) -> Option<(CStr256, Arc<RamInode>)> {
        assert!(idx >= 2);
        self.children.remove(idx - 2)
    }

    fn substitute_entry(
        &mut self,
        idx: usize,
        new_entry: (CStr256, Arc<RamInode>),
    ) -> Option<(CStr256, Arc<RamInode>)> {
        assert!(idx >= 2);
        self.children.put_at(idx - 2, new_entry)
    }

    fn visit_entry(&self, idx: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        let try_visit = |idx: &mut usize, visitor: &mut dyn DirentVisitor| -> Result<()> {
            // Read the two special entries("." and "..").
            if *idx == 0 {
                let this_inode = self.this.upgrade().unwrap();
                visitor.visit(".", this_inode.ino, this_inode.typ, *idx)?;
                *idx += 1;
            }
            if *idx == 1 {
                let parent_inode = self.parent.upgrade().unwrap();
                visitor.visit("..", parent_inode.ino, parent_inode.typ, *idx)?;
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
                visitor.visit(name.as_str().unwrap(), child.ino, child.typ, offset)?;
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
    fn new_dir(
        fs: &Arc<RamFS>,
        mode: InodeMode,
        uid: Uid,
        gid: Gid,
        parent: &Weak<RamInode>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| RamInode {
            node: RwMutex::new(Node::new_dir(
                mode,
                uid,
                gid,
                weak_self.clone(),
                parent.clone(),
            )),
            ino: fs.alloc_id(),
            typ: InodeType::Dir,
            this: weak_self.clone(),
            fs: Arc::downgrade(fs),
        })
    }

    fn new_file(fs: &Arc<RamFS>, mode: InodeMode, uid: Uid, gid: Gid) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| RamInode {
            node: RwMutex::new(Node::new_file(mode, uid, gid, weak_self.clone())),
            ino: fs.alloc_id(),
            typ: InodeType::File,
            this: weak_self.clone(),
            fs: Arc::downgrade(fs),
        })
    }

    fn new_symlink(fs: &Arc<RamFS>, mode: InodeMode, uid: Uid, gid: Gid) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| RamInode {
            node: RwMutex::new(Node::new_symlink(mode, uid, gid)),
            ino: fs.alloc_id(),
            typ: InodeType::SymLink,
            this: weak_self.clone(),
            fs: Arc::downgrade(fs),
        })
    }

    fn new_socket(fs: &Arc<RamFS>, mode: InodeMode, uid: Uid, gid: Gid) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| RamInode {
            node: RwMutex::new(Node::new_socket(mode, uid, gid)),
            ino: fs.alloc_id(),
            typ: InodeType::Socket,
            this: weak_self.clone(),
            fs: Arc::downgrade(fs),
        })
    }

    fn new_device(
        fs: &Arc<RamFS>,
        mode: InodeMode,
        uid: Uid,
        gid: Gid,
        device: Arc<dyn Device>,
    ) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| RamInode {
            node: RwMutex::new(Node::new_device(mode, uid, gid, device.clone())),
            ino: fs.alloc_id(),
            typ: InodeType::from(device.type_()),
            this: weak_self.clone(),
            fs: Arc::downgrade(fs),
        })
    }

    fn find(&self, name: &str) -> Result<Arc<Self>> {
        if self.typ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "self is not dir");
        }

        let self_inode = self.node.read();
        let (_, inode) = self_inode
            .inner
            .as_direntry()
            .unwrap()
            .get_entry(name)
            .ok_or(Error::new(Errno::ENOENT))?;
        Ok(inode)
    }
}

impl PageCacheBackend for RamInode {
    fn read_page(&self, _idx: usize, frame: &Frame) -> Result<BioWaiter> {
        // Initially, any block/page in a RamFs inode contains all zeros
        frame.writer().fill(0);
        Ok(BioWaiter::new())
    }

    fn write_page(&self, _idx: usize, _frame: &Frame) -> Result<BioWaiter> {
        // do nothing
        Ok(BioWaiter::new())
    }

    fn npages(&self) -> usize {
        self.node.read().metadata.blocks
    }
}

impl Inode for RamInode {
    fn page_cache(&self) -> Option<Vmo<Full>> {
        self.node
            .read()
            .inner
            .as_file()
            .map(|page_cache| page_cache.pages().dup())
    }

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        let read_len = {
            let self_inode = self.node.read();

            if let Some(device) = self_inode.inner.as_device() {
                device.read(buf)?
            } else {
                let Some(page_cache) = self_inode.inner.as_file() else {
                    return_errno_with_message!(Errno::EISDIR, "read is not supported");
                };
                let (offset, read_len) = {
                    let file_size = self_inode.metadata.size;
                    let start = file_size.min(offset);
                    let end = file_size.min(offset + buf.len());
                    (start, end - start)
                };
                page_cache
                    .pages()
                    .read_bytes(offset, &mut buf[..read_len])?;
                read_len
            }
        };

        self.set_atime(now());

        Ok(read_len)
    }

    fn read_direct_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        self.read_at(offset, buf)
    }

    fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        let self_inode = self.node.upread();

        if let Some(device) = self_inode.inner.as_device() {
            let device_written_len = device.write(buf)?;
            let mut self_inode = self_inode.upgrade();
            let now = now();
            self_inode.set_mtime(now);
            self_inode.set_ctime(now);
            return Ok(device_written_len);
        }

        let Some(page_cache) = self_inode.inner.as_file() else {
            return_errno_with_message!(Errno::EISDIR, "write is not supported");
        };
        let file_size = self_inode.metadata.size;
        let new_size = offset + buf.len();
        let should_expand_size = new_size > file_size;
        if should_expand_size {
            page_cache.pages().resize(new_size)?;
        }
        page_cache.pages().write_bytes(offset, buf)?;

        let mut self_inode = self_inode.upgrade();
        let now = now();
        self_inode.set_mtime(now);
        self_inode.set_ctime(now);
        if should_expand_size {
            self_inode.resize(new_size);
        }

        Ok(buf.len())
    }

    fn write_direct_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        self.write_at(offset, buf)
    }

    fn size(&self) -> usize {
        self.node.read().metadata.size
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        if self.typ != InodeType::File {
            return_errno_with_message!(Errno::EISDIR, "not regular file");
        }

        let self_inode = self.node.upread();
        let file_size = self_inode.metadata.size;
        if file_size == new_size {
            return Ok(());
        }

        let mut self_inode = self_inode.upgrade();
        self_inode.resize(new_size);
        let now = now();
        self_inode.set_mtime(now);
        self_inode.set_ctime(now);

        let self_inode = self_inode.downgrade();
        let page_cache = self_inode.inner.as_file().unwrap();
        page_cache.pages().resize(new_size)?;

        Ok(())
    }

    fn atime(&self) -> Duration {
        self.node.read().atime()
    }

    fn set_atime(&self, time: Duration) {
        self.node.write().set_atime(time)
    }

    fn mtime(&self) -> Duration {
        self.node.read().mtime()
    }

    fn set_mtime(&self, time: Duration) {
        self.node.write().set_mtime(time)
    }

    fn ctime(&self) -> Duration {
        self.node.read().ctime()
    }

    fn set_ctime(&self, time: Duration) {
        self.node.write().set_ctime(time)
    }

    fn ino(&self) -> u64 {
        self.ino
    }

    fn type_(&self) -> InodeType {
        self.typ
    }

    fn mode(&self) -> Result<InodeMode> {
        Ok(self.node.read().metadata.mode)
    }

    fn set_mode(&self, mode: InodeMode) -> Result<()> {
        let mut self_inode = self.node.write();
        self_inode.metadata.mode = mode;
        self_inode.set_ctime(now());
        Ok(())
    }

    fn owner(&self) -> Result<Uid> {
        Ok(self.node.read().metadata.uid)
    }

    fn set_owner(&self, uid: Uid) -> Result<()> {
        let mut self_inode = self.node.write();
        self_inode.metadata.uid = uid;
        self_inode.set_ctime(now());
        Ok(())
    }

    fn group(&self) -> Result<Gid> {
        Ok(self.node.read().metadata.gid)
    }

    fn set_group(&self, gid: Gid) -> Result<()> {
        let mut self_inode = self.node.write();
        self_inode.metadata.gid = gid;
        self_inode.set_ctime(now());
        Ok(())
    }

    fn mknod(
        &self,
        name: &str,
        mode: InodeMode,
        device: Arc<dyn Device>,
    ) -> Result<Arc<dyn Inode>> {
        if name.len() > NAME_MAX {
            return_errno!(Errno::ENAMETOOLONG);
        }
        if self.typ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "self is not dir");
        }

        let self_inode = self.node.upread();
        if self_inode.inner.as_direntry().unwrap().contains_entry(name) {
            return_errno_with_message!(Errno::EEXIST, "entry exists");
        }
        let device_inode = RamInode::new_device(
            &self.fs.upgrade().unwrap(),
            mode,
            Uid::new_root(),
            Gid::new_root(),
            device,
        );

        let mut self_inode = self_inode.upgrade();
        self_inode
            .inner
            .as_direntry_mut()
            .unwrap()
            .append_entry(name, device_inode.clone());
        self_inode.inc_size();
        Ok(device_inode)
    }

    fn as_device(&self) -> Option<Arc<dyn Device>> {
        self.node.read().inner.as_device().cloned()
    }

    fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<dyn Inode>> {
        if name.len() > NAME_MAX {
            return_errno!(Errno::ENAMETOOLONG);
        }
        if self.typ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "self is not dir");
        }

        let self_inode = self.node.upread();
        if self_inode.inner.as_direntry().unwrap().contains_entry(name) {
            return_errno_with_message!(Errno::EEXIST, "entry exists");
        }
        let fs = self.fs.upgrade().unwrap();
        let new_inode = match type_ {
            InodeType::File => RamInode::new_file(&fs, mode, Uid::new_root(), Gid::new_root()),
            InodeType::SymLink => {
                RamInode::new_symlink(&fs, mode, Uid::new_root(), Gid::new_root())
            }
            InodeType::Socket => RamInode::new_socket(&fs, mode, Uid::new_root(), Gid::new_root()),
            InodeType::Dir => {
                RamInode::new_dir(&fs, mode, Uid::new_root(), Gid::new_root(), &self.this)
            }
            _ => {
                panic!("unsupported inode type");
            }
        };

        let mut self_inode = self_inode.upgrade();
        if InodeType::Dir == type_ {
            self_inode.inc_nlinks();
        }
        self_inode
            .inner
            .as_direntry_mut()
            .unwrap()
            .append_entry(name, new_inode.clone());
        self_inode.inc_size();
        let now = now();
        self_inode.set_mtime(now);
        self_inode.set_ctime(now);

        Ok(new_inode)
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        if self.typ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "self is not dir");
        }

        let cnt = self
            .node
            .read()
            .inner
            .as_direntry()
            .unwrap()
            .visit_entry(offset, visitor)?;

        self.set_atime(now());

        Ok(cnt)
    }

    fn link(&self, old: &Arc<dyn Inode>, name: &str) -> Result<()> {
        if !Arc::ptr_eq(&self.fs(), &old.fs()) {
            return_errno_with_message!(Errno::EXDEV, "not same fs");
        }
        if self.typ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "self is not dir");
        }
        let old = old
            .downcast_ref::<RamInode>()
            .ok_or(Error::new(Errno::EXDEV))?;
        if old.typ == InodeType::Dir {
            return_errno_with_message!(Errno::EPERM, "old is a dir");
        }

        let mut self_inode = self.node.write();
        let self_dir = self_inode.inner.as_direntry_mut().unwrap();
        if self_dir.contains_entry(name) {
            return_errno_with_message!(Errno::EEXIST, "entry exist");
        }
        self_dir.append_entry(name, old.this.upgrade().unwrap());
        self_inode.inc_size();
        let now = now();
        self_inode.set_mtime(now);
        self_inode.set_ctime(now);
        drop(self_inode);

        let mut old_inode = old.node.write();
        old_inode.inc_nlinks();
        old_inode.set_ctime(now);

        Ok(())
    }

    fn unlink(&self, name: &str) -> Result<()> {
        if name == "." || name == ".." {
            return_errno_with_message!(Errno::EISDIR, "unlink . or ..");
        }

        let target = self.find(name)?;
        if target.typ == InodeType::Dir {
            return_errno_with_message!(Errno::EISDIR, "unlink on dir");
        }

        // When we got the lock, the dir may have been modified by another thread
        let (mut self_inode, mut target_inode) = write_lock_two_inodes(self, &target);
        let self_dir = self_inode.inner.as_direntry_mut().unwrap();
        let (idx, new_target) = self_dir.get_entry(name).ok_or(Error::new(Errno::ENOENT))?;
        if !Arc::ptr_eq(&new_target, &target) {
            return_errno!(Errno::ENOENT);
        }

        self_dir.remove_entry(idx);
        self_inode.dec_size();
        target_inode.dec_nlinks();
        let now = now();
        self_inode.set_mtime(now);
        self_inode.set_ctime(now);
        target_inode.set_ctime(now);

        Ok(())
    }

    fn rmdir(&self, name: &str) -> Result<()> {
        if name == "." {
            return_errno_with_message!(Errno::EINVAL, "rmdir on .");
        }
        if name == ".." {
            return_errno_with_message!(Errno::ENOTEMPTY, "rmdir on ..");
        }

        let target = self.find(name)?;
        if target.typ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "rmdir on not dir");
        }

        let target_inode = target.node.read();
        if !target_inode
            .inner
            .as_direntry()
            .unwrap()
            .is_empty_children()
        {
            return_errno_with_message!(Errno::ENOTEMPTY, "dir not empty");
        }
        drop(target_inode);

        // When we got the lock, the dir may have been modified by another thread
        let (mut self_inode, mut target_inode) = write_lock_two_inodes(self, &target);
        let self_dir = self_inode.inner.as_direntry_mut().unwrap();
        let (idx, new_target) = self_dir.get_entry(name).ok_or(Error::new(Errno::ENOENT))?;
        if !Arc::ptr_eq(&new_target, &target) {
            return_errno!(Errno::ENOENT);
        }
        if !target_inode
            .inner
            .as_direntry()
            .unwrap()
            .is_empty_children()
        {
            return_errno_with_message!(Errno::ENOTEMPTY, "dir not empty");
        }
        self_dir.remove_entry(idx);
        self_inode.dec_size();
        self_inode.dec_nlinks();
        let now = now();
        self_inode.set_mtime(now);
        self_inode.set_ctime(now);
        target_inode.dec_nlinks();
        target_inode.dec_nlinks();

        Ok(())
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        let inode = self.find(name)?;
        Ok(inode as _)
    }

    fn rename(&self, old_name: &str, target: &Arc<dyn Inode>, new_name: &str) -> Result<()> {
        if old_name == "." || old_name == ".." {
            return_errno_with_message!(Errno::EISDIR, "old_name is . or ..");
        }
        if new_name == "." || new_name == ".." {
            return_errno_with_message!(Errno::EISDIR, "new_name is . or ..");
        }

        let target = target
            .downcast_ref::<RamInode>()
            .ok_or(Error::new(Errno::EXDEV))?;

        if !Arc::ptr_eq(&self.fs(), &target.fs()) {
            return_errno_with_message!(Errno::EXDEV, "not same fs");
        }
        if self.typ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "self is not dir");
        }
        if target.typ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "target is not dir");
        }

        // Perform necessary checks to ensure that `dst_inode` can be replaced by `src_inode`.
        let check_replace_inode =
            |src_inode: &Arc<RamInode>, dst_inode: &Arc<RamInode>| -> Result<()> {
                if src_inode.ino == dst_inode.ino {
                    return Ok(());
                }

                match (src_inode.typ, dst_inode.typ) {
                    (InodeType::Dir, InodeType::Dir) => {
                        if !dst_inode
                            .node
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
                Ok(())
            };

        // Rename in the same directory
        if self.ino == target.ino {
            let mut self_inode = self.node.write();
            let self_dir = self_inode.inner.as_direntry_mut().unwrap();
            let (src_idx, src_inode) = self_dir
                .get_entry(old_name)
                .ok_or(Error::new(Errno::ENOENT))?;
            let is_dir = src_inode.typ == InodeType::Dir;

            if let Some((dst_idx, dst_inode)) = self_dir.get_entry(new_name) {
                check_replace_inode(&src_inode, &dst_inode)?;
                self_dir.remove_entry(dst_idx);
                self_dir.substitute_entry(src_idx, (CStr256::from(new_name), src_inode.clone()));
                self_inode.dec_size();
                if is_dir {
                    self_inode.dec_nlinks();
                }
                let now = now();
                self_inode.set_mtime(now);
                self_inode.set_ctime(now);
                drop(self_inode);
                dst_inode.set_ctime(now);
                src_inode.set_ctime(now);
            } else {
                self_dir.substitute_entry(src_idx, (CStr256::from(new_name), src_inode.clone()));
                let now = now();
                self_inode.set_mtime(now);
                self_inode.set_ctime(now);
                drop(self_inode);
                src_inode.set_ctime(now);
            }
        }
        // Or rename across different directories
        else {
            let (mut self_inode, mut target_inode) = write_lock_two_inodes(self, target);
            let self_inode_arc = self.this.upgrade().unwrap();
            let target_inode_arc = target.this.upgrade().unwrap();
            let self_dir = self_inode.inner.as_direntry_mut().unwrap();
            let (src_idx, src_inode) = self_dir
                .get_entry(old_name)
                .ok_or(Error::new(Errno::ENOENT))?;
            // Avoid renaming a directory to a subdirectory of itself
            if Arc::ptr_eq(&src_inode, &target_inode_arc) {
                return_errno!(Errno::EINVAL);
            }
            let is_dir = src_inode.typ == InodeType::Dir;

            let target_dir = target_inode.inner.as_direntry_mut().unwrap();
            if let Some((dst_idx, dst_inode)) = target_dir.get_entry(new_name) {
                // Avoid renaming a subdirectory to a directory.
                if Arc::ptr_eq(&self_inode_arc, &dst_inode) {
                    return_errno!(Errno::ENOTEMPTY);
                }
                check_replace_inode(&src_inode, &dst_inode)?;
                self_dir.remove_entry(src_idx);
                target_dir.remove_entry(dst_idx);
                target_dir.append_entry(new_name, src_inode.clone());
                self_inode.dec_size();
                if is_dir {
                    self_inode.dec_nlinks();
                }
                let now = now();
                self_inode.set_mtime(now);
                self_inode.set_ctime(now);
                target_inode.set_mtime(now);
                target_inode.set_ctime(now);
                drop(self_inode);
                drop(target_inode);
                dst_inode.set_ctime(now);
                src_inode.set_ctime(now);
            } else {
                self_dir.remove_entry(src_idx);
                target_dir.append_entry(new_name, src_inode.clone());
                self_inode.dec_size();
                target_inode.inc_size();
                if is_dir {
                    self_inode.dec_nlinks();
                    target_inode.inc_nlinks();
                }
                let now = now();
                self_inode.set_mtime(now);
                self_inode.set_ctime(now);
                target_inode.set_mtime(now);
                target_inode.set_ctime(now);
                drop(self_inode);
                drop(target_inode);
                src_inode.set_ctime(now);
            }

            if is_dir {
                src_inode
                    .node
                    .write()
                    .inner
                    .as_direntry_mut()
                    .unwrap()
                    .set_parent(target.this.clone());
            }
        }
        Ok(())
    }

    fn read_link(&self) -> Result<String> {
        if self.typ != InodeType::SymLink {
            return_errno_with_message!(Errno::EINVAL, "self is not symlink");
        }

        let self_inode = self.node.read();
        let link = self_inode.inner.as_symlink().unwrap();
        Ok(String::from(link))
    }

    fn write_link(&self, target: &str) -> Result<()> {
        if self.typ != InodeType::SymLink {
            return_errno_with_message!(Errno::EINVAL, "self is not symlink");
        }

        let mut self_inode = self.node.write();
        let link = self_inode.inner.as_symlink_mut().unwrap();
        *link = String::from(target);
        // Symlink's metadata.blocks should be 0, so just set the size.
        self_inode.metadata.size = target.len();
        Ok(())
    }

    fn metadata(&self) -> Metadata {
        let self_inode = self.node.read();
        Metadata {
            dev: self.fs.upgrade().unwrap().device_id(),
            ino: self.ino as _,
            size: self_inode.metadata.size,
            blk_size: BLOCK_SIZE,
            blocks: self_inode.metadata.blocks,
            atime: self_inode.metadata.atime,
            mtime: self_inode.metadata.mtime,
            ctime: self_inode.metadata.ctime,
            type_: self.typ,
            mode: self_inode.metadata.mode,
            nlinks: self_inode.metadata.nlinks,
            uid: self_inode.metadata.uid,
            gid: self_inode.metadata.gid,
            rdev: {
                if let Some(device) = self_inode.inner.as_device() {
                    device.id().into()
                } else {
                    0
                }
            },
        }
    }

    fn poll(&self, mask: IoEvents, poller: Option<&mut Poller>) -> IoEvents {
        if let Some(device) = self.node.read().inner.as_device() {
            device.poll(mask, poller)
        } else {
            let events = IoEvents::IN | IoEvents::OUT;
            events & mask
        }
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        Weak::upgrade(&self.fs).unwrap()
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        if let Some(device) = self.node.read().inner.as_device() {
            return device.ioctl(cmd, arg);
        }
        return_errno_with_message!(Errno::EINVAL, "ioctl is not supported");
    }
}

fn write_lock_two_inodes<'a>(
    this: &'a RamInode,
    other: &'a RamInode,
) -> (RwMutexWriteGuard<'a, Node>, RwMutexWriteGuard<'a, Node>) {
    if this.ino < other.ino {
        let this = this.node.write();
        let other = other.node.write();
        (this, other)
    } else {
        let other = other.node.write();
        let this = this.node.write();
        (this, other)
    }
}

fn now() -> Duration {
    RealTimeCoarseClock::get().read_time()
}
