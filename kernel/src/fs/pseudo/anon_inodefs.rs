// SPDX-License-Identifier: MPL-2.0

use alloc::{
    format,
    sync::{Arc, Weak},
};
use core::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use ostd::{mm::PAGE_SIZE, sync::SpinLock};
use spin::Once;

use crate::{
    fs::{
        path::{Dentry, MountNode},
        pseudo::alloc_pseudo_superblock,
        utils::{FileSystem, FsFlags, Inode, InodeMode, InodeType, Metadata, SuperBlock},
    },
    prelude::*,
    process::{Gid, Uid},
    time::{clocks::RealTimeCoarseClock, Clock},
};

/// Magic number.
const ANON_INODE_FS_MAGIC: u64 = 0x09041934;
/// Root Inode ID.
const ANON_INODE_ROOT_INO: u64 = 1;

static ANON_INODE: Once<Arc<dyn Inode>> = Once::new();
static ANON_INODE_DENTRY: Once<Arc<Dentry>> = Once::new();

pub struct AnonInodeFs {
    // The super block
    sb: SuperBlock,
    // Root inode
    root: Arc<dyn Inode>,
    /// An inode allocator
    inode_allocator: AtomicU64,
}

impl AnonInodeFs {
    pub fn new() -> Arc<Self> {
        let fs = Arc::new_cyclic(|weak_fs| Self {
            sb: alloc_pseudo_superblock(ANON_INODE_FS_MAGIC),
            root: Arc::new(AnonInode {
                metadata: SpinLock::new(InodeMeta::new(
                    InodeMode::from_bits_truncate(0o777),
                    Uid::new_root(),
                    Gid::new_root(),
                )),
                ino: ANON_INODE_ROOT_INO,
                typ: InodeType::Dir,
                fs: weak_fs.clone(),
            }),
            inode_allocator: AtomicU64::new(ANON_INODE_ROOT_INO + 1),
        });
        ANON_INODE.call_once(|| AnonInode::new(&fs, InodeType::File));
        ANON_INODE_DENTRY
            .call_once(|| Arc::new(Dentry::new_fs_root(MountNode::new_root(fs.clone()))));
        fs
    }

    pub fn alloc_id(&self) -> u64 {
        self.inode_allocator.fetch_add(1, Ordering::SeqCst)
    }
}

impl FileSystem for AnonInodeFs {
    fn sync(&self) -> Result<()> {
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

/// An inode of `AnonInodeFs`
struct AnonInode {
    /// Inode metadata
    metadata: SpinLock<InodeMeta>,
    /// Inode number
    ino: u64,
    /// Type of the inode
    typ: InodeType,
    /// Reference to fs
    fs: Weak<AnonInodeFs>,
}

impl AnonInode {
    fn new(fs: &Arc<AnonInodeFs>, typ: InodeType) -> Arc<Self> {
        Arc::new(Self {
            metadata: SpinLock::new(InodeMeta::new(
                InodeMode::from_bits_truncate(0x600),
                Uid::new_root(),
                Gid::new_root(),
            )),
            ino: fs.alloc_id(),
            typ,
            fs: Arc::downgrade(fs),
        })
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

    pub fn set_atime(&mut self, time: Duration) {
        self.atime = time;
    }

    pub fn set_mtime(&mut self, time: Duration) {
        self.mtime = time;
    }

    pub fn set_ctime(&mut self, time: Duration) {
        self.ctime = time;
    }
}

fn now() -> Duration {
    RealTimeCoarseClock::get().read_time()
}

impl Inode for AnonInode {
    fn size(&self) -> usize {
        self.metadata.lock().size
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        self.metadata.lock().size = new_size;
        Ok(())
    }

    fn metadata(&self) -> Metadata {
        let inode_metadata = self.metadata.lock();
        Metadata {
            dev: 0,
            ino: self.ino as _,
            size: inode_metadata.size,
            blk_size: PAGE_SIZE,
            blocks: inode_metadata.blocks,
            atime: inode_metadata.atime,
            mtime: inode_metadata.mtime,
            ctime: inode_metadata.ctime,
            type_: self.typ,
            mode: inode_metadata.mode,
            nlinks: inode_metadata.nlinks,
            uid: inode_metadata.uid,
            gid: inode_metadata.gid,
            rdev: 0,
        }
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

    fn owner(&self) -> crate::Result<Uid> {
        Ok(self.metadata.lock().uid)
    }

    fn set_owner(&self, uid: Uid) -> crate::Result<()> {
        let mut inode_meta = self.metadata.lock();
        inode_meta.uid = uid;
        inode_meta.set_ctime(now());
        Ok(())
    }

    fn group(&self) -> crate::Result<Gid> {
        Ok(self.metadata.lock().gid)
    }

    fn set_group(&self, gid: Gid) -> crate::Result<()> {
        let mut inode_meta = self.metadata.lock();
        inode_meta.gid = gid;
        inode_meta.set_ctime(now());
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

    fn fs(&self) -> Arc<dyn FileSystem> {
        Weak::upgrade(&self.fs).unwrap()
    }
}

pub fn alloc_anon_dentry(name: &str) -> Result<Dentry> {
    let name = format!("anon_inode:{}", name);
    anon_inode_dentry().new_pseudo_dentry(&name, anon_inode().clone())
}

fn anon_inode_dentry() -> &'static Arc<Dentry> {
    ANON_INODE_DENTRY.get().unwrap()
}

fn anon_inode() -> &'static Arc<dyn Inode> {
    ANON_INODE.get().unwrap()
}
