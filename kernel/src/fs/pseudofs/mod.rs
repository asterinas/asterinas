// SPDX-License-Identifier: MPL-2.0

//! The infrastructure of pseudo FSes along with some naive implementations of pseudo FSes.
//!
//! Pseudo FSes are those that have no physical storage.
//! Some pseudo FSes are trivial,
//! e.g., [`PipeFs`], [`SockFs`], [`PidfdFs`], and [`AnonInodeFs`].
//! Such trivial pseudo FSes are directly implemented in this module
//! using a generic pseudo FS implementation called `NaivePseudoFs`.
//!
//! On the other hand, some pseudo FSes are quite complex,
//! including [`RamFs`], `ProcFs`, and `SysFs`. Just to name a few.
//! They are implemented in their own modules.
//!
//! Pseudo FSes are not backed by physical storage devices,
//! but they still need to be assigned container device IDs
//! (see [`Metadata::container_dev_id`]).
//! [Linux's behavior](https://elixir.bootlin.com/linux/v6.19.3/source/fs/super.c#L1254)
//! is to assign _anonymous device IDs_ (whose major IDs are 0).
//! As such, this module provides a device ID allocator called [`allocator::DEVICE_ID_ALLOCATOR`]
//! to allocate and free anonymous device IDs.
//!
//! [`RamFs`]: crate::fs::ramfs::RamFs

use core::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

pub use anon_inodefs::AnonInodeFs;
use device_id::DeviceId;
pub use pidfdfs::PidfdFs;
pub(super) use pipefs::PipeFs;
use pipefs::PipeFsType;
pub use sockfs::SockFs;
use sockfs::SockFsType;
use spin::Once;

use super::utils::{Extension, InodeIo, StatusFlags};
use crate::{
    fs::{
        inode_handle::FileIo,
        utils::{
            AccessMode, FileSystem, FsEventSubscriberStats, Inode, InodeMode, InodeType, Metadata,
            NAME_MAX, SuperBlock, mkmod,
        },
    },
    prelude::*,
    process::{Gid, Uid},
    time::clocks::RealTimeCoarseClock,
};

mod allocator;
mod anon_inodefs;
mod pidfdfs;
mod pipefs;
mod sockfs;
pub use allocator::DEVICE_ID_ALLOCATOR;

/// A pseudo file system that manages pseudo inodes, such as pipe inodes and socket inodes.
pub struct PseudoFs {
    name: &'static str,
    sb: SuperBlock,
    root: Arc<dyn Inode>,
    inode_allocator: AtomicU64,
    fs_event_subscriber_stats: FsEventSubscriberStats,
}

impl FileSystem for PseudoFs {
    fn name(&self) -> &'static str {
        self.name
    }

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

    fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats {
        &self.fs_event_subscriber_stats
    }
}

impl PseudoFs {
    /// Returns a reference to the singleton pseudo file system.
    fn singleton(
        fs: &'static Once<Arc<Self>>,
        name: &'static str,
        magic: u64,
    ) -> &'static Arc<Self> {
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/libfs.c#L659-L689>
        fs.call_once(|| {
            let dev_id = allocator::DEVICE_ID_ALLOCATOR
                .get()
                .unwrap()
                .allocate()
                .expect("no device ID is available for pseudofs");
            Arc::new_cyclic(|weak_fs: &Weak<Self>| Self {
                name,
                sb: SuperBlock::new(magic, aster_block::BLOCK_SIZE, NAME_MAX, dev_id),
                root: Arc::new(PseudoInode::new(
                    ROOT_INO,
                    PseudoInodeType::Root,
                    mkmod!(u+rw),
                    Uid::new_root(),
                    Gid::new_root(),
                    weak_fs.clone(),
                    dev_id,
                )),
                inode_allocator: AtomicU64::new(ROOT_INO + 1),
                fs_event_subscriber_stats: FsEventSubscriberStats::new(),
            })
        })
    }

    pub fn alloc_inode(
        self: &Arc<Self>,
        type_: PseudoInodeType,
        mode: InodeMode,
        uid: Uid,
        gid: Gid,
    ) -> PseudoInode {
        PseudoInode::new(
            self.alloc_id(),
            type_,
            mode,
            uid,
            gid,
            Arc::downgrade(self),
            self.sb.container_dev_id,
        )
    }

    fn alloc_id(&self) -> u64 {
        self.inode_allocator.fetch_add(1, Ordering::Relaxed)
    }
}

pub(super) fn init() {
    super::registry::register(&PipeFsType).unwrap();
    super::registry::register(&SockFsType).unwrap();
    // Note: `AnonInodeFs` does not need to be registered in the FS registry.
    // Reference: <https://elixir.bootlin.com/linux/v6.16.5/A/ident/anon_inode_fs_type>
    allocator::DEVICE_ID_ALLOCATOR.call_once(allocator::DeviceIdAllocator::new);
}

/// Root Inode ID.
const ROOT_INO: u64 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PseudoInodeType {
    Root,
    Pipe,
    Socket,
    AnonInode,
    Pidfd,
}

impl From<PseudoInodeType> for InodeType {
    fn from(pseudo_type: PseudoInodeType) -> Self {
        match pseudo_type {
            PseudoInodeType::Root => InodeType::Dir,
            PseudoInodeType::Pipe => InodeType::NamedPipe,
            PseudoInodeType::Socket => InodeType::Socket,
            PseudoInodeType::AnonInode => InodeType::Unknown,
            PseudoInodeType::Pidfd => InodeType::Unknown,
        }
    }
}

/// A pseudo inode that does not correspond to any real path in the file system.
pub struct PseudoInode {
    metadata: SpinLock<Metadata>,
    extension: Extension,
    fs: Weak<PseudoFs>,
    is_anon: bool,
}

impl PseudoInode {
    fn new(
        ino: u64,
        type_: PseudoInodeType,
        mode: InodeMode,
        uid: Uid,
        gid: Gid,
        fs: Weak<PseudoFs>,
        dev_id: DeviceId,
    ) -> Self {
        let now = now();
        let type_ = InodeType::from(type_);

        let metadata = Metadata {
            ino,
            size: 0,
            optimal_block_size: aster_block::BLOCK_SIZE,
            nr_sectors_allocated: 0,
            last_access_at: now,
            last_modify_at: now,
            last_meta_change_at: now,
            type_,
            mode,
            nr_hard_links: 1,
            uid,
            gid,
            container_dev_id: dev_id,
            self_dev_id: None,
        };

        PseudoInode {
            metadata: SpinLock::new(metadata),
            extension: Extension::new(),
            fs,
            is_anon: type_ == InodeType::Unknown,
        }
    }
}

impl InodeIo for PseudoInode {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(
            Errno::ESPIPE,
            "pseudo inodes cannot be read at a specific offset"
        );
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(
            Errno::ESPIPE,
            "pseudo inodes cannot be written at a specific offset"
        );
    }
}

impl Inode for PseudoInode {
    fn size(&self) -> usize {
        self.metadata.lock().size
    }

    fn resize(&self, _new_size: usize) -> Result<()> {
        return_errno_with_message!(Errno::EINVAL, "pseudo inodes can not be resized");
    }

    fn metadata(&self) -> Metadata {
        *self.metadata.lock()
    }

    fn extension(&self) -> &Extension {
        &self.extension
    }

    fn ino(&self) -> u64 {
        self.metadata.lock().ino
    }

    fn type_(&self) -> InodeType {
        self.metadata.lock().type_
    }

    fn mode(&self) -> Result<InodeMode> {
        Ok(self.metadata.lock().mode)
    }

    fn set_mode(&self, mode: InodeMode) -> Result<()> {
        if self.is_anon {
            return_errno_with_message!(
                Errno::EOPNOTSUPP,
                "the mode of anonymous inodes cannot be changed"
            );
        }

        let mut meta = self.metadata.lock();
        meta.mode = mode;
        meta.last_meta_change_at = now();
        Ok(())
    }

    fn owner(&self) -> Result<Uid> {
        Ok(self.metadata.lock().uid)
    }

    fn set_owner(&self, uid: Uid) -> Result<()> {
        let mut meta = self.metadata.lock();
        meta.uid = uid;
        meta.last_meta_change_at = now();
        Ok(())
    }

    fn group(&self) -> Result<Gid> {
        Ok(self.metadata.lock().gid)
    }

    fn set_group(&self, gid: Gid) -> Result<()> {
        let mut meta = self.metadata.lock();
        meta.gid = gid;
        meta.last_meta_change_at = now();
        Ok(())
    }

    fn atime(&self) -> Duration {
        self.metadata.lock().last_access_at
    }

    fn set_atime(&self, time: Duration) {
        self.metadata.lock().last_access_at = time;
    }

    fn mtime(&self) -> Duration {
        self.metadata.lock().last_modify_at
    }

    fn set_mtime(&self, time: Duration) {
        self.metadata.lock().last_modify_at = time;
    }

    fn ctime(&self) -> Duration {
        self.metadata.lock().last_meta_change_at
    }

    fn set_ctime(&self, time: Duration) {
        self.metadata.lock().last_meta_change_at = time;
    }

    fn open(
        &self,
        _access_mode: AccessMode,
        _status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn FileIo>>> {
        Some(Err(Error::with_message(
            Errno::ENXIO,
            "the pseudo inode is not re-openable",
        )))
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.fs.upgrade().unwrap()
    }
}

fn now() -> Duration {
    RealTimeCoarseClock::get().read_time()
}
