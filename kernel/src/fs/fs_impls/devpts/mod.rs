// SPDX-License-Identifier: MPL-2.0

#![expect(unused_variables)]

use core::time::Duration;

use aster_util::slot_vec::SlotVec;
use id_alloc::IdAlloc;

pub use self::ptmx::Ptmx;
use self::slave::PtySlaveInode;
use super::utils::{Extension, MknodType, StatusFlags};
use crate::{
    device::PtyMaster,
    fs::{
        device::{Device, DeviceType},
        registry::{FsProperties, FsType},
        utils::{
            DirEntryVecExt, DirentVisitor, FileSystem, FsEventSubscriberStats, FsFlags, Inode,
            InodeIo, InodeMode, InodeType, Metadata, NAME_MAX, SuperBlock, mkmod,
        },
    },
    prelude::*,
    process::{Gid, Uid},
};

mod ptmx;
mod slave;

const DEVPTS_MAGIC: u64 = 0x1cd1;
const BLOCK_SIZE: usize = 1024;

const ROOT_INO: u64 = 1;
const PTMX_INO: u64 = 2;
const FIRST_SLAVE_INO: u64 = 3;

/// The max number of pty pairs.
const MAX_PTY_NUM: usize = 4096;

/// Devpts(device pseudo terminal filesystem) is a virtual filesystem.
///
/// It is normally mounted at "/dev/pts" and contains solely devices files which
/// represent slaves to the multiplexing master located at "/dev/ptmx".
///
/// Actually, the "/dev/ptmx" is a symlink to the real device at "/dev/pts/ptmx".
pub struct DevPts {
    sb: SuperBlock,
    root: Arc<RootInode>,
    index_alloc: Mutex<IdAlloc>,
    fs_event_subscriber_stats: FsEventSubscriberStats,
    this: Weak<Self>,
}

impl DevPts {
    pub fn new() -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            sb: SuperBlock::new(DEVPTS_MAGIC, BLOCK_SIZE, NAME_MAX),
            root: RootInode::new(weak_self.clone()),
            index_alloc: Mutex::new(IdAlloc::with_capacity(MAX_PTY_NUM)),
            fs_event_subscriber_stats: FsEventSubscriberStats::new(),
            this: weak_self.clone(),
        })
    }

    /// Create the master and slave pair.
    fn create_master_slave_pair(&self) -> Result<(Box<PtyMaster>, Arc<PtySlaveInode>)> {
        let index = self
            .index_alloc
            .lock()
            .alloc()
            .ok_or_else(|| Error::with_message(Errno::EIO, "cannot alloc index"))?;

        let (master, slave) = crate::device::new_pty_pair(index as u32, self.root.ptmx.clone())?;

        let slave_inode = PtySlaveInode::new(slave, self.this.clone());
        self.root
            .slaves
            .write()
            .put_entry_if_not_found(&index.to_string(), || slave_inode.clone());

        Ok((master, slave_inode))
    }

    /// Remove the slave from fs.
    ///
    /// This is called when the master is being dropped.
    pub fn remove_slave(&self, index: u32) -> Option<Arc<dyn Inode>> {
        let (_, removed_slave) = self
            .root
            .slaves
            .write()
            .remove_entry_by_name(&index.to_string())?;
        self.index_alloc.lock().free(index as usize);
        Some(removed_slave)
    }
}

impl FileSystem for DevPts {
    fn name(&self) -> &'static str {
        "devpts"
    }

    fn sync(&self) -> Result<()> {
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

struct DevPtsType;

impl FsType for DevPtsType {
    fn name(&self) -> &'static str {
        "devpts"
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
        Ok(DevPts::new())
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        None
    }
}

pub(super) fn init() {
    super::registry::register(&DevPtsType).unwrap();
}

struct RootInode {
    ptmx: Arc<Ptmx>,
    slaves: RwLock<SlotVec<(String, Arc<dyn Inode>)>>,
    metadata: RwLock<Metadata>,
    extension: Extension,
    fs: Weak<DevPts>,
}

impl RootInode {
    pub fn new(fs: Weak<DevPts>) -> Arc<Self> {
        Arc::new(Self {
            ptmx: Ptmx::new(fs.clone()),
            slaves: RwLock::new(SlotVec::new()),
            metadata: RwLock::new(Metadata::new_dir(ROOT_INO, mkmod!(a+rx, u+w), BLOCK_SIZE)),
            extension: Extension::new(),
            fs,
        })
    }
}

impl InodeIo for RootInode {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        Err(Error::new(Errno::EISDIR))
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        Err(Error::new(Errno::EISDIR))
    }
}

impl Inode for RootInode {
    fn size(&self) -> usize {
        self.metadata.read().size
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        Err(Error::new(Errno::EISDIR))
    }

    fn metadata(&self) -> Metadata {
        *self.metadata.read()
    }

    fn extension(&self) -> &Extension {
        &self.extension
    }

    fn ino(&self) -> u64 {
        self.metadata.read().ino as _
    }

    fn type_(&self) -> InodeType {
        self.metadata.read().type_
    }

    fn mode(&self) -> Result<InodeMode> {
        Ok(self.metadata.read().mode)
    }

    fn set_mode(&self, mode: InodeMode) -> Result<()> {
        self.metadata.write().mode = mode;
        Ok(())
    }

    fn owner(&self) -> Result<Uid> {
        Ok(self.metadata.read().uid)
    }

    fn set_owner(&self, uid: Uid) -> Result<()> {
        self.metadata.write().uid = uid;
        Ok(())
    }

    fn group(&self) -> Result<Gid> {
        Ok(self.metadata.read().gid)
    }

    fn set_group(&self, gid: Gid) -> Result<()> {
        self.metadata.write().gid = gid;
        Ok(())
    }

    fn atime(&self) -> Duration {
        self.metadata.read().atime
    }

    fn set_atime(&self, time: Duration) {
        self.metadata.write().atime = time;
    }

    fn mtime(&self) -> Duration {
        self.metadata.read().mtime
    }

    fn set_mtime(&self, time: Duration) {
        self.metadata.write().mtime = time;
    }

    fn ctime(&self) -> Duration {
        self.metadata.read().ctime
    }

    fn set_ctime(&self, time: Duration) {
        self.metadata.write().ctime = time;
    }

    fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::EPERM))
    }

    fn mknod(&self, name: &str, mode: InodeMode, type_: MknodType) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::EPERM))
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        let try_readdir = |offset: &mut usize, visitor: &mut dyn DirentVisitor| -> Result<()> {
            // Read the 3 special entries.
            if *offset == 0 {
                visitor.visit(".", self.ino(), self.type_(), *offset)?;
                *offset += 1;
            }
            if *offset == 1 {
                visitor.visit("..", self.ino(), self.type_(), *offset)?;
                *offset += 1;
            }
            if *offset == 2 {
                visitor.visit("ptmx", self.ptmx.ino(), self.ptmx.type_(), *offset)?;
                *offset += 1;
            }

            // Read the slaves.
            let slaves = self.slaves.read();
            let start_offset = *offset;
            for (idx, (name, node)) in slaves
                .idxes_and_items()
                .map(|(idx, (name, node))| (idx + 3, (name, node)))
                .skip_while(|(idx, _)| idx < &start_offset)
            {
                visitor.visit(name.as_ref(), node.ino(), node.type_(), idx)?;
                *offset = idx + 1;
            }
            Ok(())
        };

        let mut iterate_offset = offset;
        match try_readdir(&mut iterate_offset, visitor) {
            Err(e) if offset == iterate_offset => Err(e),
            _ => Ok(iterate_offset - offset),
        }
    }

    fn link(&self, old: &Arc<dyn Inode>, name: &str) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    fn unlink(&self, name: &str) -> Result<()> {
        match name {
            "." | ".." => {
                return_errno_with_message!(Errno::EISDIR, "the devpts directory cannot be unlinked")
            }
            "ptmx" => return_errno_with_message!(Errno::EPERM, "the ptmx inode cannot be unlinked"),
            slave => {
                if self.slaves.read().find_entry_by_name(slave).is_none() {
                    return_errno_with_message!(Errno::ENOENT, "the slave inode does not exist");
                }
                return_errno_with_message!(Errno::EPERM, "the slave inode cannot be unlinked");
            }
        }
    }

    fn rmdir(&self, name: &str) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        let inode = match name {
            "." | ".." => self.fs().root_inode(),
            // Call the "open" method of ptmx to create a master and slave pair.
            "ptmx" => self.ptmx.clone(),
            slave => self
                .slaves
                .read()
                .find_entry_by_name(slave)
                .cloned()
                .ok_or(Error::new(Errno::ENOENT))?,
        };
        Ok(inode)
    }

    fn rename(&self, old_name: &str, target: &Arc<dyn Inode>, new_name: &str) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.fs.upgrade().unwrap()
    }

    fn is_dentry_cacheable(&self) -> bool {
        false
    }
}
