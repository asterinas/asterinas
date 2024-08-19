// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use core::time::Duration;

use aster_util::slot_vec::SlotVec;
use id_alloc::IdAlloc;

use self::{ptmx::Ptmx, slave::PtySlaveInode};
use super::utils::MknodType;
use crate::{
    device::PtyMaster,
    fs::{
        device::{Device, DeviceId, DeviceType},
        utils::{
            DirentVisitor, FileSystem, FsFlags, Inode, InodeMode, InodeType, IoctlCmd, Metadata,
            SuperBlock, NAME_MAX,
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
    this: Weak<Self>,
}

impl DevPts {
    pub fn new() -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            sb: SuperBlock::new(DEVPTS_MAGIC, BLOCK_SIZE, NAME_MAX),
            root: RootInode::new(weak_self.clone()),
            index_alloc: Mutex::new(IdAlloc::with_capacity(MAX_PTY_NUM)),
            this: weak_self.clone(),
        })
    }

    /// Create the master and slave pair.
    fn create_master_slave_pair(&self) -> Result<(Arc<PtyMaster>, Arc<PtySlaveInode>)> {
        let index = self
            .index_alloc
            .lock()
            .alloc()
            .ok_or_else(|| Error::with_message(Errno::EIO, "cannot alloc index"))?;

        let (master, slave) = crate::device::new_pty_pair(index as u32, self.root.ptmx.clone())?;

        let slave_inode = PtySlaveInode::new(slave, self.this.clone());
        self.root.add_slave(index.to_string(), slave_inode.clone());

        Ok((master, slave_inode))
    }

    /// Remove the slave from fs.
    ///
    /// This is called when the master is being dropped.
    pub fn remove_slave(&self, index: u32) -> Option<Arc<PtySlaveInode>> {
        let removed_slave = self.root.remove_slave(&index.to_string());
        if removed_slave.is_some() {
            self.index_alloc.lock().free(index as usize);
        }
        removed_slave
    }
}

impl FileSystem for DevPts {
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
        FsFlags::empty()
    }
}

struct RootInode {
    ptmx: Arc<Ptmx>,
    slaves: RwLock<SlotVec<(String, Arc<PtySlaveInode>)>>,
    metadata: RwLock<Metadata>,
    fs: Weak<DevPts>,
}

impl RootInode {
    pub fn new(fs: Weak<DevPts>) -> Arc<Self> {
        Arc::new(Self {
            ptmx: Ptmx::new(fs.clone()),
            slaves: RwLock::new(SlotVec::new()),
            metadata: RwLock::new(Metadata::new_dir(
                ROOT_INO,
                InodeMode::from_bits_truncate(0o755),
                BLOCK_SIZE,
            )),
            fs,
        })
    }

    fn add_slave(&self, name: String, slave: Arc<PtySlaveInode>) {
        self.slaves.write().put((name, slave));
    }

    fn remove_slave(&self, name: &str) -> Option<Arc<PtySlaveInode>> {
        let removed_slave = {
            let mut slaves = self.slaves.write();
            let pos = slaves
                .idxes_and_items()
                .find(|(_, (child, _))| child == name)
                .map(|(pos, _)| pos);
            match pos {
                None => {
                    return None;
                }
                Some(pos) => slaves.remove(pos).map(|(_, node)| node).unwrap(),
            }
        };
        Some(removed_slave)
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
        Err(Error::new(Errno::EPERM))
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
                .idxes_and_items()
                .find(|(_, (child_name, _))| child_name == slave)
                .map(|(_, (_, node))| node.clone())
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
}
