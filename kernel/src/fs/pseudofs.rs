// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use spin::Once;

use super::utils::{Extension, InodeIo, StatusFlags};
use crate::{
    fs::{
        registry::{FsProperties, FsType},
        utils::{
            FileSystem, FsEventSubscriberStats, FsFlags, Inode, InodeMode, InodeType, Metadata,
            NAME_MAX, SuperBlock, mkmod,
        },
    },
    prelude::*,
    process::{Gid, Uid},
    time::clocks::RealTimeCoarseClock,
};

/// A pseudo file system that manages pseudo inodes, such as pipe inodes and socket inodes.
pub struct PseudoFs {
    name: &'static str,
    sb: SuperBlock,
    root: Arc<dyn Inode>,
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
        fs.call_once(|| {
            Arc::new_cyclic(|weak_fs: &Weak<Self>| Self {
                name,
                sb: SuperBlock::new(magic, aster_block::BLOCK_SIZE, NAME_MAX),
                root: Arc::new(PseudoInode::new(
                    0,
                    InodeType::Unknown,
                    mkmod!(u+rw),
                    Uid::new_root(),
                    Gid::new_root(),
                    aster_block::BLOCK_SIZE,
                    weak_fs.clone(),
                )),
                fs_event_subscriber_stats: FsEventSubscriberStats::new(),
            })
        })
    }
}

/// Returns the singleton instance of the anonymous pipe file system.
pub fn pipefs_singleton() -> &'static Arc<PseudoFs> {
    static PIPEFS: Once<Arc<PseudoFs>> = Once::new();

    PseudoFs::singleton(&PIPEFS, "pipefs", PIPEFS_MAGIC)
}

/// Returns the singleton instance of the socket file system.
pub fn sockfs_singleton() -> &'static Arc<PseudoFs> {
    static SOCKFS: Once<Arc<PseudoFs>> = Once::new();

    PseudoFs::singleton(&SOCKFS, "sockfs", SOCKFS_MAGIC)
}

/// Returns the singleton instance of the anonymous inode file system.
fn anon_inodefs_singleton() -> &'static Arc<PseudoFs> {
    static ANON_INODEFS: Once<Arc<PseudoFs>> = Once::new();

    PseudoFs::singleton(&ANON_INODEFS, "anon_inodefs", ANON_INODEFS_MAGIC)
}

/// Returns the shared inode of the anonymous inode file system singleton.
//
// Some members of anon_inodefs (such as epollfd, eventfd, timerfd, etc.) share
// the same inode. The sharing is not only within the same category (e.g., two
// epollfds share the same inode) but also across different categories (e.g.,
// an epollfd and a timerfd share the same inode). Even across namespaces, this
// inode is still shared. Although this Linux behavior is a bit odd, we keep it
// for compatibility.
//
// A small subset of members in anon_inodefs (i.e., userfaultfd, io_uring, and
// kvm_guest_memfd) have their own dedicated inodes. We need to support creating
// independent inodes within anon_inodefs for them in the future.
//
// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/anon_inodes.c#L153-L164>
pub fn anon_inodefs_shared_inode() -> &'static Arc<dyn Inode> {
    &anon_inodefs_singleton().root
}

pub(super) fn init() {
    super::registry::register(&PipeFsType).unwrap();
    super::registry::register(&SockFsType).unwrap();
    // Note: `AnonInodeFs` does not need to be registered in the FS registry.
    // Reference: <https://elixir.bootlin.com/linux/v6.16.5/A/ident/anon_inode_fs_type>
}

pub(super) struct PipeFsType;

impl FsType for PipeFsType {
    fn name(&self) -> &'static str {
        "pipefs"
    }

    fn properties(&self) -> FsProperties {
        FsProperties::empty()
    }

    fn create(
        &self,
        _flags: FsFlags,
        _args: Option<CString>,
        _disk: Option<Arc<dyn aster_block::BlockDevice>>,
    ) -> Result<Arc<dyn FileSystem>> {
        return_errno_with_message!(Errno::EINVAL, "pipefs cannot be mounted");
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        None
    }
}

pub(super) struct SockFsType;

impl FsType for SockFsType {
    fn name(&self) -> &'static str {
        "sockfs"
    }

    fn properties(&self) -> FsProperties {
        FsProperties::empty()
    }

    fn create(
        &self,
        _flags: FsFlags,
        _args: Option<CString>,
        _disk: Option<Arc<dyn aster_block::BlockDevice>>,
    ) -> Result<Arc<dyn FileSystem>> {
        return_errno_with_message!(Errno::EINVAL, "sockfs cannot be mounted");
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        None
    }
}

// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/include/uapi/linux/magic.h#L87>
const PIPEFS_MAGIC: u64 = 0x50495045;
// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/include/uapi/linux/magic.h#L89>
const SOCKFS_MAGIC: u64 = 0x534F434B;
// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/include/uapi/linux/magic.h#L93>
const ANON_INODEFS_MAGIC: u64 = 0x09041934;

/// A pseudo inode that does not correspond to any real path in the file system.
pub struct PseudoInode {
    metadata: SpinLock<Metadata>,
    extension: Extension,
    fs: Weak<PseudoFs>,
}

impl PseudoInode {
    pub fn new(
        ino: u64,
        type_: InodeType,
        mode: InodeMode,
        uid: Uid,
        gid: Gid,
        blk_size: usize,
        fs: Weak<PseudoFs>,
    ) -> Self {
        let now = now();
        let metadata = Metadata {
            dev: 0,
            ino,
            size: 0,
            blk_size,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            type_,
            mode,
            nlinks: 1,
            uid,
            gid,
            rdev: 0,
        };
        PseudoInode {
            metadata: SpinLock::new(metadata),
            extension: Extension::new(),
            fs,
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
        let mut meta = self.metadata.lock();
        meta.mode = mode;
        meta.ctime = now();
        Ok(())
    }

    fn owner(&self) -> Result<Uid> {
        Ok(self.metadata.lock().uid)
    }

    fn set_owner(&self, uid: Uid) -> Result<()> {
        let mut meta = self.metadata.lock();
        meta.uid = uid;
        meta.ctime = now();
        Ok(())
    }

    fn group(&self) -> Result<Gid> {
        Ok(self.metadata.lock().gid)
    }

    fn set_group(&self, gid: Gid) -> Result<()> {
        let mut meta = self.metadata.lock();
        meta.gid = gid;
        meta.ctime = now();
        Ok(())
    }

    fn atime(&self) -> Duration {
        self.metadata.lock().atime
    }

    fn set_atime(&self, time: Duration) {
        self.metadata.lock().atime = time;
    }

    fn mtime(&self) -> Duration {
        self.metadata.lock().mtime
    }

    fn set_mtime(&self, time: Duration) {
        self.metadata.lock().mtime = time;
    }

    fn ctime(&self) -> Duration {
        self.metadata.lock().ctime
    }

    fn set_ctime(&self, time: Duration) {
        self.metadata.lock().ctime = time;
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.fs.upgrade().unwrap()
    }
}

fn now() -> Duration {
    RealTimeCoarseClock::get().read_time()
}
