// SPDX-License-Identifier: MPL-2.0

use core::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use align_ext::AlignExt;
use aster_block::bio::BioWaiter;
use aster_rights::Full;
use aster_util::slot_vec::SlotVec;
use hashbrown::HashMap;
use ostd::{
    mm::{Frame, VmIo},
    sync::{PreemptDisabled, RwLockWriteGuard},
};

use super::*;
use crate::{
    events::IoEvents,
    fs::{
        device::Device,
        file_handle::FileLike,
        named_pipe::NamedPipe,
        utils::{
            CStr256, DirentVisitor, Extension, FallocMode, FileSystem, FsFlags, Inode, InodeMode,
            InodeType, IoctlCmd, Metadata, MknodType, PageCache, PageCacheBackend, SuperBlock,
        },
    },
    prelude::*,
    process::{signal::PollHandle, Gid, Uid},
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
                inner: Inner::new_dir(weak_root.clone(), weak_root.clone()),
                metadata: SpinLock::new(InodeMeta::new_dir(
                    InodeMode::from_bits_truncate(0o755),
                    Uid::new_root(),
                    Gid::new_root(),
                )),
                ino: ROOT_INO,
                typ: InodeType::Dir,
                this: weak_root.clone(),
                fs: weak_fs.clone(),
                extension: Extension::new(),
            }),
            inode_allocator: AtomicU64::new(ROOT_INO + 1),
        })
    }

    fn alloc_id(&self) -> u64 {
        self.inode_allocator.fetch_add(1, Ordering::SeqCst)
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

/// An inode of `RamFs`.
struct RamInode {
    /// Inode inner specifics
    inner: Inner,
    /// Inode metadata
    metadata: SpinLock<InodeMeta>,
    /// Inode number
    ino: u64,
    /// Type of the inode
    typ: InodeType,
    /// Reference to self
    this: Weak<RamInode>,
    /// Reference to fs
    fs: Weak<RamFS>,
    /// Extensions
    extension: Extension,
}

/// Inode inner specifics.
#[allow(clippy::large_enum_variant)]
enum Inner {
    Dir(RwLock<DirEntry>),
    File(PageCache),
    SymLink(SpinLock<String>),
    Device(Arc<dyn Device>),
    Socket,
    NamedPipe(NamedPipe),
}

impl Inner {
    pub fn new_dir(this: Weak<RamInode>, parent: Weak<RamInode>) -> Self {
        Self::Dir(RwLock::new(DirEntry::new(this, parent)))
    }

    pub fn new_file(this: Weak<RamInode>) -> Self {
        Self::File(PageCache::new(this).unwrap())
    }

    pub fn new_symlink() -> Self {
        Self::SymLink(SpinLock::new(String::from("")))
    }

    pub fn new_device(device: Arc<dyn Device>) -> Self {
        Self::Device(device)
    }

    pub fn new_socket() -> Self {
        Self::Socket
    }

    pub fn new_named_pipe() -> Self {
        Self::NamedPipe(NamedPipe::new().unwrap())
    }

    fn as_direntry(&self) -> Option<&RwLock<DirEntry>> {
        match self {
            Self::Dir(dir_entry) => Some(dir_entry),
            _ => None,
        }
    }

    fn as_file(&self) -> Option<&PageCache> {
        match self {
            Self::File(page_cache) => Some(page_cache),
            _ => None,
        }
    }

    fn as_symlink(&self) -> Option<&SpinLock<String>> {
        match self {
            Self::SymLink(link) => Some(link),
            _ => None,
        }
    }

    fn as_device(&self) -> Option<&Arc<dyn Device>> {
        match self {
            Self::Device(device) => Some(device),
            _ => None,
        }
    }

    fn as_named_pipe(&self) -> Option<&NamedPipe> {
        match self {
            Self::NamedPipe(pipe) => Some(pipe),
            _ => None,
        }
    }
}

/// Inode metadata.
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
            size: NUM_SPECIAL_ENTRIES,
            blocks: 1,
            atime: now,
            mtime: now,
            ctime: now,
            mode,
            nlinks: NUM_SPECIAL_ENTRIES,
            uid,
            gid,
        }
    }

    pub fn resize(&mut self, new_size: usize) {
        self.size = new_size;
        self.blocks = new_size.align_up(BLOCK_SIZE) / BLOCK_SIZE;
    }

    pub fn inc_size(&mut self) {
        self.size += 1;
        self.blocks = self.size.align_up(BLOCK_SIZE) / BLOCK_SIZE;
    }

    pub fn dec_size(&mut self) {
        debug_assert!(self.size > 0);
        self.size -= 1;
        self.blocks = self.size.align_up(BLOCK_SIZE) / BLOCK_SIZE;
    }

    pub fn set_atime(&mut self, time: Duration) {
        self.atime = time;
    }

    pub fn set_mtime(&mut self, time: Duration) {
        self.mtime = time;
    }

    pub fn set_ctime(&mut self, time: Duration) {
        self.ctime = time;
    }

    pub fn inc_nlinks(&mut self) {
        self.nlinks += 1;
    }

    pub fn dec_nlinks(&mut self) {
        debug_assert!(self.nlinks > 0);
        self.nlinks -= 1;
    }
}

/// Represents a directory entry within a `RamInode`.
struct DirEntry {
    children: SlotVec<(CStr256, Arc<RamInode>)>,
    idx_map: HashMap<CStr256, usize>, // Used to accelerate indexing in `children`
    this: Weak<RamInode>,
    parent: Weak<RamInode>,
}

// Every directory has two special entries: "." and "..".
const NUM_SPECIAL_ENTRIES: usize = 2;

impl DirEntry {
    fn new(this: Weak<RamInode>, parent: Weak<RamInode>) -> Self {
        Self {
            children: SlotVec::new(),
            idx_map: HashMap::new(),
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
            self.idx_map.contains_key(name.as_bytes())
        }
    }

    fn get_entry(&self, name: &str) -> Option<(usize, Arc<RamInode>)> {
        if name == "." {
            Some((0, self.this.upgrade().unwrap()))
        } else if name == ".." {
            Some((1, self.parent.upgrade().unwrap()))
        } else {
            let idx = *self.idx_map.get(name.as_bytes())?;
            let target_inode = self
                .children
                .get(idx)
                .map(|(name_cstr256, inode)| {
                    debug_assert_eq!(name, name_cstr256.as_str().unwrap());
                    inode.clone()
                })
                .unwrap();
            Some((idx + NUM_SPECIAL_ENTRIES, target_inode))
        }
    }

    fn append_entry(&mut self, name: &str, inode: Arc<RamInode>) -> usize {
        let name = CStr256::from(name);
        let idx = self.children.put((name, inode));
        self.idx_map.insert(name, idx);
        idx
    }

    fn remove_entry(&mut self, idx: usize) -> Option<(CStr256, Arc<RamInode>)> {
        assert!(idx >= NUM_SPECIAL_ENTRIES);
        let removed = self.children.remove(idx - NUM_SPECIAL_ENTRIES)?;
        self.idx_map.remove(&removed.0);
        Some(removed)
    }

    fn substitute_entry(
        &mut self,
        idx: usize,
        new_entry: (CStr256, Arc<RamInode>),
    ) -> Option<(CStr256, Arc<RamInode>)> {
        assert!(idx >= NUM_SPECIAL_ENTRIES);
        let new_name = new_entry.0;
        let idx_children = idx - NUM_SPECIAL_ENTRIES;

        let substitute = self.children.put_at(idx_children, new_entry)?;
        let removed = self.idx_map.remove(&substitute.0);
        debug_assert_eq!(removed.unwrap(), idx_children);
        self.idx_map.insert(new_name, idx_children);
        Some(substitute)
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
            for (offset_children, (name, child)) in self
                .children
                .idxes_and_items()
                .skip_while(|(offset, _)| offset + NUM_SPECIAL_ENTRIES < start_idx)
            {
                let offset = offset_children + NUM_SPECIAL_ENTRIES;
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
            inner: Inner::new_dir(weak_self.clone(), parent.clone()),
            metadata: SpinLock::new(InodeMeta::new_dir(mode, uid, gid)),
            ino: fs.alloc_id(),
            typ: InodeType::Dir,
            this: weak_self.clone(),
            fs: Arc::downgrade(fs),
            extension: Extension::new(),
        })
    }

    fn new_file(fs: &Arc<RamFS>, mode: InodeMode, uid: Uid, gid: Gid) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| RamInode {
            inner: Inner::new_file(weak_self.clone()),
            metadata: SpinLock::new(InodeMeta::new(mode, uid, gid)),
            ino: fs.alloc_id(),
            typ: InodeType::File,
            this: weak_self.clone(),
            fs: Arc::downgrade(fs),
            extension: Extension::new(),
        })
    }

    fn new_symlink(fs: &Arc<RamFS>, mode: InodeMode, uid: Uid, gid: Gid) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| RamInode {
            inner: Inner::new_symlink(),
            metadata: SpinLock::new(InodeMeta::new(mode, uid, gid)),
            ino: fs.alloc_id(),
            typ: InodeType::SymLink,
            this: weak_self.clone(),
            fs: Arc::downgrade(fs),
            extension: Extension::new(),
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
            inner: Inner::new_device(device.clone()),
            metadata: SpinLock::new(InodeMeta::new(mode, uid, gid)),
            ino: fs.alloc_id(),
            typ: InodeType::from(device.type_()),
            this: weak_self.clone(),
            fs: Arc::downgrade(fs),
            extension: Extension::new(),
        })
    }

    fn new_socket(fs: &Arc<RamFS>, mode: InodeMode, uid: Uid, gid: Gid) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| RamInode {
            inner: Inner::new_socket(),
            metadata: SpinLock::new(InodeMeta::new(mode, uid, gid)),
            ino: fs.alloc_id(),
            typ: InodeType::Socket,
            this: weak_self.clone(),
            fs: Arc::downgrade(fs),
            extension: Extension::new(),
        })
    }

    fn new_named_pipe(fs: &Arc<RamFS>, mode: InodeMode, uid: Uid, gid: Gid) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| RamInode {
            inner: Inner::new_named_pipe(),
            metadata: SpinLock::new(InodeMeta::new(mode, uid, gid)),
            ino: fs.alloc_id(),
            typ: InodeType::NamedPipe,
            this: weak_self.clone(),
            fs: Arc::downgrade(fs),
            extension: Extension::new(),
        })
    }

    fn find(&self, name: &str) -> Result<Arc<Self>> {
        if self.typ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "self is not dir");
        }

        let (_, inode) = self
            .inner
            .as_direntry()
            .unwrap()
            .read()
            .get_entry(name)
            .ok_or(Error::new(Errno::ENOENT))?;
        Ok(inode)
    }
}

impl PageCacheBackend for RamInode {
    fn read_page_async(&self, _idx: usize, frame: &Frame) -> Result<BioWaiter> {
        // Initially, any block/page in a RamFs inode contains all zeros
        frame
            .writer()
            .to_fallible()
            .fill_zeros(frame.size())
            .unwrap();
        Ok(BioWaiter::new())
    }

    fn write_page_async(&self, _idx: usize, _frame: &Frame) -> Result<BioWaiter> {
        // do nothing
        Ok(BioWaiter::new())
    }

    fn npages(&self) -> usize {
        self.metadata.lock().blocks
    }
}

impl Inode for RamInode {
    fn page_cache(&self) -> Option<Vmo<Full>> {
        self.inner
            .as_file()
            .map(|page_cache| page_cache.pages().dup())
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let read_len = {
            match &self.inner {
                Inner::File(page_cache) => {
                    let (offset, read_len) = {
                        let file_size = self.size();
                        let start = file_size.min(offset);
                        let end = file_size.min(offset + writer.avail());
                        (start, end - start)
                    };
                    page_cache.pages().read(offset, writer)?;
                    read_len
                }
                Inner::Device(device) => {
                    device.read(writer)?
                    // Typically, devices like "/dev/zero" or "/dev/null" do not require modifying
                    // timestamps here. Please adjust this behavior accordingly if there are special devices.
                }
                Inner::NamedPipe(named_pipe) => named_pipe.read(writer)?,
                _ => return_errno_with_message!(Errno::EISDIR, "read is not supported"),
            }
        };

        if self.typ == InodeType::File {
            self.set_atime(now());
        }
        Ok(read_len)
    }

    fn read_direct_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        self.read_at(offset, writer)
    }

    fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        let written_len = match self.typ {
            InodeType::File => {
                let page_cache = self.inner.as_file().unwrap();

                let file_size = self.size();
                let write_len = reader.remain();
                let new_size = offset + write_len;
                let should_expand_size = new_size > file_size;
                let new_size_aligned = new_size.align_up(BLOCK_SIZE);
                if should_expand_size {
                    page_cache.resize(new_size_aligned)?;
                }
                page_cache.pages().write(offset, reader)?;

                let now = now();
                let mut inode_meta = self.metadata.lock();
                inode_meta.set_mtime(now);
                inode_meta.set_ctime(now);
                if should_expand_size {
                    inode_meta.size = new_size;
                    inode_meta.blocks = new_size_aligned / BLOCK_SIZE;
                }
                write_len
            }
            InodeType::CharDevice | InodeType::BlockDevice => {
                let device = self.inner.as_device().unwrap();
                device.write(reader)?
                // Typically, devices like "/dev/zero" or "/dev/null" do not require modifying
                // timestamps here. Please adjust this behavior accordingly if there are special devices.
            }
            InodeType::NamedPipe => {
                let named_pipe = self.inner.as_named_pipe().unwrap();
                named_pipe.write(reader)?
            }
            _ => return_errno_with_message!(Errno::EISDIR, "write is not supported"),
        };
        Ok(written_len)
    }

    fn write_direct_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        self.write_at(offset, reader)
    }

    fn size(&self) -> usize {
        self.metadata.lock().size
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        if self.typ != InodeType::File {
            return_errno_with_message!(Errno::EISDIR, "not regular file");
        }

        let file_size = self.size();
        if file_size == new_size {
            return Ok(());
        }

        let page_cache = self.inner.as_file().unwrap();
        page_cache.resize(new_size)?;

        let now = now();
        let mut inode_meta = self.metadata.lock();
        inode_meta.set_mtime(now);
        inode_meta.set_ctime(now);
        inode_meta.resize(new_size);
        Ok(())
    }

    fn atime(&self) -> Duration {
        self.metadata.lock().atime
    }

    fn set_atime(&self, time: Duration) {
        self.metadata.lock().set_atime(time);
    }

    fn mtime(&self) -> Duration {
        self.metadata.lock().mtime
    }

    fn set_mtime(&self, time: Duration) {
        self.metadata.lock().set_mtime(time);
    }

    fn ctime(&self) -> Duration {
        self.metadata.lock().ctime
    }

    fn set_ctime(&self, time: Duration) {
        self.metadata.lock().set_ctime(time);
    }

    fn ino(&self) -> u64 {
        self.ino
    }

    fn type_(&self) -> InodeType {
        self.typ
    }

    fn mode(&self) -> Result<InodeMode> {
        Ok(self.metadata.lock().mode)
    }

    fn set_mode(&self, mode: InodeMode) -> Result<()> {
        let mut inode_meta = self.metadata.lock();
        inode_meta.mode = mode;
        inode_meta.set_ctime(now());
        Ok(())
    }

    fn owner(&self) -> Result<Uid> {
        Ok(self.metadata.lock().uid)
    }

    fn set_owner(&self, uid: Uid) -> Result<()> {
        let mut inode_meta = self.metadata.lock();
        inode_meta.uid = uid;
        inode_meta.set_ctime(now());
        Ok(())
    }

    fn group(&self) -> Result<Gid> {
        Ok(self.metadata.lock().gid)
    }

    fn set_group(&self, gid: Gid) -> Result<()> {
        let mut inode_meta = self.metadata.lock();
        inode_meta.gid = gid;
        inode_meta.set_ctime(now());
        Ok(())
    }

    fn mknod(&self, name: &str, mode: InodeMode, type_: MknodType) -> Result<Arc<dyn Inode>> {
        if name.len() > NAME_MAX {
            return_errno!(Errno::ENAMETOOLONG);
        }
        if self.typ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "self is not dir");
        }

        let self_dir = self.inner.as_direntry().unwrap().upread();
        if self_dir.contains_entry(name) {
            return_errno_with_message!(Errno::EEXIST, "entry exists");
        }

        let new_inode = match type_ {
            MknodType::CharDeviceNode(device) | MknodType::BlockDeviceNode(device) => {
                RamInode::new_device(
                    &self.fs.upgrade().unwrap(),
                    mode,
                    Uid::new_root(),
                    Gid::new_root(),
                    device,
                )
            }
            MknodType::NamedPipeNode => RamInode::new_named_pipe(
                &self.fs.upgrade().unwrap(),
                mode,
                Uid::new_root(),
                Gid::new_root(),
            ),
        };

        let mut self_dir = self_dir.upgrade();
        self_dir.append_entry(name, new_inode.clone());
        drop(self_dir);

        self.metadata.lock().inc_size();
        Ok(new_inode)
    }

    fn as_device(&self) -> Option<Arc<dyn Device>> {
        if !self.typ.is_device() {
            return None;
        }
        self.inner.as_device().cloned()
    }

    fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<dyn Inode>> {
        if name.len() > NAME_MAX {
            return_errno!(Errno::ENAMETOOLONG);
        }
        if self.typ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "self is not dir");
        }

        let self_dir = self.inner.as_direntry().unwrap().upread();
        if self_dir.contains_entry(name) {
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

        let mut self_dir = self_dir.upgrade();
        self_dir.append_entry(name, new_inode.clone());
        drop(self_dir);

        let now = now();
        let mut inode_meta = self.metadata.lock();
        inode_meta.set_mtime(now);
        inode_meta.set_ctime(now);
        inode_meta.inc_size();
        if type_ == InodeType::Dir {
            inode_meta.inc_nlinks();
        }

        Ok(new_inode)
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        if self.typ != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "self is not dir");
        }

        let cnt = self
            .inner
            .as_direntry()
            .unwrap()
            .read()
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

        let mut self_dir = self.inner.as_direntry().unwrap().write();
        if self_dir.contains_entry(name) {
            return_errno_with_message!(Errno::EEXIST, "entry exist");
        }
        self_dir.append_entry(name, old.this.upgrade().unwrap());
        drop(self_dir);

        let now = now();
        let mut self_meta = self.metadata.lock();
        self_meta.set_mtime(now);
        self_meta.set_ctime(now);
        self_meta.inc_size();
        drop(self_meta);

        let mut old_meta = old.metadata.lock();
        old_meta.inc_nlinks();
        old_meta.set_ctime(now);

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
        let mut self_dir = self.inner.as_direntry().unwrap().write();
        let (idx, new_target) = self_dir.get_entry(name).ok_or(Error::new(Errno::ENOENT))?;
        if !Arc::ptr_eq(&new_target, &target) {
            return_errno!(Errno::ENOENT);
        }
        self_dir.remove_entry(idx);
        drop(self_dir);

        let now = now();
        let mut self_meta = self.metadata.lock();
        self_meta.dec_size();
        self_meta.set_mtime(now);
        self_meta.set_ctime(now);
        drop(self_meta);
        let mut target_meta = target.metadata.lock();
        target_meta.dec_nlinks();
        target_meta.set_ctime(now);

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

        // When we got the lock, the dir may have been modified by another thread
        let (mut self_dir, target_dir) = write_lock_two_direntries_by_ino(
            (self.ino, self.inner.as_direntry().unwrap()),
            (target.ino, target.inner.as_direntry().unwrap()),
        );
        if !target_dir.is_empty_children() {
            return_errno_with_message!(Errno::ENOTEMPTY, "dir not empty");
        }
        let (idx, new_target) = self_dir.get_entry(name).ok_or(Error::new(Errno::ENOENT))?;
        if !Arc::ptr_eq(&new_target, &target) {
            return_errno!(Errno::ENOENT);
        }
        self_dir.remove_entry(idx);
        drop(self_dir);
        drop(target_dir);

        let now = now();
        let mut self_meta = self.metadata.lock();
        self_meta.dec_size();
        self_meta.dec_nlinks();
        self_meta.set_mtime(now);
        self_meta.set_ctime(now);
        drop(self_meta);
        let mut target_meta = target.metadata.lock();
        target_meta.dec_nlinks();
        target_meta.dec_nlinks();

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
                            .inner
                            .as_direntry()
                            .unwrap()
                            .read()
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
            let mut self_dir = self.inner.as_direntry().unwrap().write();
            let (src_idx, src_inode) = self_dir
                .get_entry(old_name)
                .ok_or(Error::new(Errno::ENOENT))?;
            let is_dir = src_inode.typ == InodeType::Dir;

            if let Some((dst_idx, dst_inode)) = self_dir.get_entry(new_name) {
                check_replace_inode(&src_inode, &dst_inode)?;
                self_dir.remove_entry(dst_idx);
                self_dir.substitute_entry(src_idx, (CStr256::from(new_name), src_inode.clone()));
                drop(self_dir);

                let now = now();
                let mut self_meta = self.metadata.lock();
                self_meta.dec_size();
                if is_dir {
                    self_meta.dec_nlinks();
                }
                self_meta.set_mtime(now);
                self_meta.set_ctime(now);
                drop(self_meta);
                src_inode.set_ctime(now);
                dst_inode.set_ctime(now);
            } else {
                self_dir.substitute_entry(src_idx, (CStr256::from(new_name), src_inode.clone()));
                drop(self_dir);
                let now = now();
                let mut self_meta = self.metadata.lock();
                self_meta.set_mtime(now);
                self_meta.set_ctime(now);
                drop(self_meta);
                src_inode.set_ctime(now);
            }
        }
        // Or rename across different directories
        else {
            let (mut self_dir, mut target_dir) = write_lock_two_direntries_by_ino(
                (self.ino, self.inner.as_direntry().unwrap()),
                (target.ino, target.inner.as_direntry().unwrap()),
            );
            let self_inode_arc = self.this.upgrade().unwrap();
            let target_inode_arc = target.this.upgrade().unwrap();
            let (src_idx, src_inode) = self_dir
                .get_entry(old_name)
                .ok_or(Error::new(Errno::ENOENT))?;
            // Avoid renaming a directory to a subdirectory of itself
            if Arc::ptr_eq(&src_inode, &target_inode_arc) {
                return_errno!(Errno::EINVAL);
            }
            let is_dir = src_inode.typ == InodeType::Dir;

            if let Some((dst_idx, dst_inode)) = target_dir.get_entry(new_name) {
                // Avoid renaming a subdirectory to a directory.
                if Arc::ptr_eq(&self_inode_arc, &dst_inode) {
                    return_errno!(Errno::ENOTEMPTY);
                }
                check_replace_inode(&src_inode, &dst_inode)?;
                self_dir.remove_entry(src_idx);
                target_dir.remove_entry(dst_idx);
                target_dir.append_entry(new_name, src_inode.clone());
                drop(self_dir);
                drop(target_dir);

                let now = now();
                let mut self_meta = self.metadata.lock();
                self_meta.dec_size();
                if is_dir {
                    self_meta.dec_nlinks();
                }
                self_meta.set_mtime(now);
                self_meta.set_ctime(now);
                drop(self_meta);
                let mut target_meta = target.metadata.lock();
                target_meta.set_mtime(now);
                target_meta.set_ctime(now);
                drop(target_meta);
                dst_inode.set_ctime(now);
                src_inode.set_ctime(now);
            } else {
                self_dir.remove_entry(src_idx);
                target_dir.append_entry(new_name, src_inode.clone());
                drop(self_dir);
                drop(target_dir);

                let now = now();
                let mut self_meta = self.metadata.lock();
                self_meta.dec_size();
                if is_dir {
                    self_meta.dec_nlinks();
                }
                self_meta.set_mtime(now);
                self_meta.set_ctime(now);
                drop(self_meta);

                let mut target_meta = target.metadata.lock();
                target_meta.inc_size();
                if is_dir {
                    target_meta.inc_nlinks();
                }
                target_meta.set_mtime(now);
                target_meta.set_ctime(now);
                drop(target_meta);
                src_inode.set_ctime(now);
            }

            if is_dir {
                src_inode
                    .inner
                    .as_direntry()
                    .unwrap()
                    .write()
                    .set_parent(target.this.clone());
            }
        }
        Ok(())
    }

    fn read_link(&self) -> Result<String> {
        if self.typ != InodeType::SymLink {
            return_errno_with_message!(Errno::EINVAL, "self is not symlink");
        }

        let link = self.inner.as_symlink().unwrap().lock();
        Ok(link.clone())
    }

    fn write_link(&self, target: &str) -> Result<()> {
        if self.typ != InodeType::SymLink {
            return_errno_with_message!(Errno::EINVAL, "self is not symlink");
        }

        let mut link = self.inner.as_symlink().unwrap().lock();
        *link = String::from(target);
        drop(link);

        // Symlink's metadata.blocks should be 0, so just set the size.
        self.metadata.lock().size = target.len();
        Ok(())
    }

    fn metadata(&self) -> Metadata {
        let rdev = self
            .inner
            .as_device()
            .map(|device| device.id().into())
            .unwrap_or(0);
        let inode_metadata = self.metadata.lock();
        Metadata {
            dev: 0,
            ino: self.ino as _,
            size: inode_metadata.size,
            blk_size: BLOCK_SIZE,
            blocks: inode_metadata.blocks,
            atime: inode_metadata.atime,
            mtime: inode_metadata.mtime,
            ctime: inode_metadata.ctime,
            type_: self.typ,
            mode: inode_metadata.mode,
            nlinks: inode_metadata.nlinks,
            uid: inode_metadata.uid,
            gid: inode_metadata.gid,
            rdev,
        }
    }

    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        if !self.typ.is_device() {
            return (IoEvents::IN | IoEvents::OUT) & mask;
        }

        let device = self
            .inner
            .as_device()
            .expect("[Internal error] self.typ is device, while self.inner is not");
        device.poll(mask, poller)
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        Weak::upgrade(&self.fs).unwrap()
    }

    fn fallocate(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()> {
        if self.typ != InodeType::File {
            return_errno_with_message!(Errno::EISDIR, "not regular file");
        }

        // The support for flags is consistent with Linux
        match mode {
            FallocMode::Allocate => {
                let new_size = offset + len;
                if new_size > self.size() {
                    self.resize(new_size)?;
                }
                Ok(())
            }
            FallocMode::AllocateKeepSize => {
                // Do nothing
                Ok(())
            }
            FallocMode::PunchHoleKeepSize => {
                let file_size = self.size();
                if offset >= file_size {
                    return Ok(());
                }
                let range = offset..file_size.min(offset + len);
                // TODO: Think of a more light-weight approach
                self.inner.as_file().unwrap().fill_zeros(range)
            }
            _ => {
                return_errno_with_message!(
                    Errno::EOPNOTSUPP,
                    "fallocate with the specified flags is not supported"
                );
            }
        }
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        if let Some(device) = self.inner.as_device() {
            return device.ioctl(cmd, arg);
        }
        return_errno_with_message!(Errno::EINVAL, "ioctl is not supported");
    }

    fn is_seekable(&self) -> bool {
        !matches!(
            self.typ,
            InodeType::NamedPipe | InodeType::CharDevice | InodeType::Dir | InodeType::Socket
        )
    }

    fn extension(&self) -> Option<&Extension> {
        Some(&self.extension)
    }
}

fn write_lock_two_direntries_by_ino<'a>(
    this: (u64, &'a RwLock<DirEntry>),
    other: (u64, &'a RwLock<DirEntry>),
) -> (
    RwLockWriteGuard<'a, DirEntry, PreemptDisabled>,
    RwLockWriteGuard<'a, DirEntry, PreemptDisabled>,
) {
    if this.0 < other.0 {
        let this = this.1.write();
        let other = other.1.write();
        (this, other)
    } else {
        let other = other.1.write();
        let this = this.1.write();
        (this, other)
    }
}

fn now() -> Duration {
    RealTimeCoarseClock::get().read_time()
}
