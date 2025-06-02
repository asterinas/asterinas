// SPDX-License-Identifier: MPL-2.0

//! Shared memory.

use alloc::sync::Arc;
use core::time::Duration;

use align_ext::AlignExt;
use ostd::{
    mm::PAGE_SIZE,
    sync::{PreemptDisabled, RwLock, RwLockReadGuard},
};

use crate::{
    fs::{
        ramfs::RamInode,
        utils::{Inode, InodeMode},
    },
    prelude::*,
    process::{Gid, Pid, Uid},
    time::clocks::RealTimeCoarseClock,
    vm::vmo::{Vmo, VmoRightsOp},
};

mod ipc_types;
mod manager;

pub use ipc_types::{IpcPerm, ShmidDs};
pub use manager::{SharedMemManager, SHM_OBJ_MANAGER};

pub const SHMMIN: usize = 1; // Minimum shared segment size in bytes
pub const SHMMAX: usize = usize::MAX - (1 << 24); // Maximum shared segment size in bytes
pub const SHMLBA: usize = PAGE_SIZE; // Shared memory segment alignment

/// Initialize the shared memory subsystem
pub fn init() {
    SHM_OBJ_MANAGER.call_once(SharedMemManager::new);
}

/// Shared memory object.
pub struct SharedMemObj {
    /// Shared memory files.
    inner: Arc<RamInode>,
    /// The reference count for the shared memory object.
    nlinks: RwLock<u32>,
    /// The metadata of the shared memory object.
    metadata: SpinLock<SharedMemMeta>,
}

impl Debug for SharedMemObj {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "SharedMemObj {{ shmid = {}, nlinks = {}, size = {}, metadata = {:?} }}",
            self.shmid(),
            *self.nlinks(),
            self.size(),
            self.metadata.lock()
        )
    }
}

#[derive(Debug)]
pub struct SharedMemMeta {
    /// Indicates if the shared memory object is anonymous
    /// That is, it was created without a key.
    is_anonymous: bool,

    /// Indicates if the reference count is zero, deleting the object.
    shm_deleted: bool,

    /// Name of the shared memory key.
    shm_key: u32,

    /// Size of the shared memory object.
    shm_size: usize,

    /// Last attach time
    shm_atime: Duration,
    /// Last detach time
    shm_dtime: Duration,
    /// Last change time
    shm_ctime: Duration,

    /// Pid of last operator
    shm_lpid: Pid,
    /// Pid of creator
    shm_cpid: Pid,
}

impl SharedMemObj {
    /// Creates a new shared memory object.
    pub fn new(shm_file: Arc<RamInode>, key: u32, is_anony: bool, size: usize, cpid: Pid) -> Self {
        Self {
            inner: shm_file,
            nlinks: RwLock::new(0),
            metadata: SpinLock::new(SharedMemMeta::new(key, is_anony, size, cpid)),
        }
    }

    /// Get the link count of the shared memory object.
    pub fn nlinks(&self) -> RwLockReadGuard<u32, PreemptDisabled> {
        self.nlinks.read()
    }

    /// Returns the key of the shared memory object.
    pub fn key(&self) -> u32 {
        self.metadata.lock().shm_key
    }

    /// Returns whether the shared memory object is anonymous.
    pub fn is_anonymous(&self) -> bool {
        self.metadata.lock().is_anonymous
    }

    /// Increases the reference count of the shared memory object.
    pub fn set_attached(&self, lpid: Pid) -> u32 {
        let now = RealTimeCoarseClock::get().read_time();
        let mut meta = self.metadata.lock();
        meta.set_shm_atime(now);
        meta.set_shm_lpid(lpid);
        let mut nlinks = self.nlinks.write();
        *nlinks += 1;
        *nlinks
    }

    /// Decreases the reference count of the shared memory object.
    pub fn set_detached(&self, lpid: Pid) -> u32 {
        let now = RealTimeCoarseClock::get().read_time();
        let mut meta = self.metadata.lock();
        meta.set_shm_dtime(now);
        meta.set_shm_lpid(lpid);
        let mut nlinks = self.nlinks.write();
        *nlinks -= 1;
        *nlinks
    }

    /// Returns the shared memory id.
    pub fn shmid(&self) -> u64 {
        let inode: Arc<dyn Inode> = self.inner.clone() as Arc<dyn Inode>;
        inode.ino()
    }

    /// Returns whether the shared memory object should be deleted when number
    /// of attach is 0.
    pub fn should_be_deleted(&self) -> bool {
        self.metadata.lock().shm_deleted
    }

    /// Sets the shared memory object as deleted.
    pub fn set_deleted(&self) {
        let mut meta = self.metadata.lock();
        meta.set_deleted();
    }

    /// Return the size of the shared memory object.
    pub fn size(&self) -> usize {
        self.metadata.lock().shm_size
    }

    /// Sets the size of the shared memory object.
    pub fn set_size(&mut self, size: usize) -> Result<()> {
        let mut meta = self.metadata.lock();
        meta.shm_size = size;
        self.inner.resize(size.align_up(PAGE_SIZE))?;
        Ok(())
    }

    /// Return the mode of the shared memory object.
    pub fn mode(&self) -> Result<InodeMode> {
        let inode: Arc<dyn Inode> = self.inner.clone() as Arc<dyn Inode>;
        inode.mode()
    }

    /// Sets the mode of the shared memory object.
    pub fn set_mode(&self, mode: InodeMode) -> Result<()> {
        self.inner.set_mode(mode)
    }

    /// Returns the VMO of the shared memory object.
    pub fn vmo(&self) -> Result<Vmo> {
        let inode: Arc<dyn Inode> = self.inner.clone() as Arc<dyn Inode>;
        let vmo = inode
            .page_cache()
            .ok_or(Error::with_message(
                Errno::EBADF,
                "File does not have page cache",
            ))?
            .to_dyn();
        Ok(vmo)
    }

    /// Sets the attributes of the shared memory object.
    pub fn set_attributes(&self, mode: InodeMode, uid: u32, gid: u32) -> Result<()> {
        self.set_mode(mode)?;
        let inode: Arc<dyn Inode> = self.inner.clone() as Arc<dyn Inode>;
        inode.set_owner(Uid::from(uid))?;
        inode.set_group(Gid::from(gid))?;

        let now = RealTimeCoarseClock::get().read_time();
        self.metadata.lock().set_shm_ctime(now);

        Ok(())
    }

    /// Returns the attributes of the shared memory object.
    pub fn get_attributes(&self) -> Result<ShmidDs> {
        let meta = self.metadata.lock();
        let inode: Arc<dyn Inode> = self.inner.clone() as Arc<dyn Inode>;

        Ok(ShmidDs {
            shm_perm: IpcPerm {
                key: meta.shm_key as i32,
                uid: u32::from(inode.owner()?),
                gid: u32::from(inode.group()?),
                cuid: meta.shm_cpid,
                cgid: u32::from(inode.group()?),
                mode: self.mode()?.bits(),
                seq: 0,
                _pad2: 0,
                _glibc_reserved1: 0,
                _glibc_reserved2: 0,
            },
            shm_segsz: meta.shm_size,
            shm_atime: meta.shm_atime.as_secs() as i64,
            shm_dtime: meta.shm_dtime.as_secs() as i64,
            shm_ctime: meta.shm_ctime.as_secs() as i64,
            shm_cpid: meta.shm_cpid as i32,
            shm_lpid: meta.shm_lpid as i32,
            shm_nattch: *self.nlinks() as u64,
            _glibc_reserved5: 0,
            _glibc_reserved6: 0,
        })
    }

    /// Returns the uid of the shared memory object.
    pub fn uid(&self) -> Result<u32> {
        let inode: Arc<dyn Inode> = self.inner.clone() as Arc<dyn Inode>;
        Ok(u32::from(inode.owner()?))
    }

    /// Returns the gid of the shared memory object.
    pub fn gid(&self) -> Result<u32> {
        let inode: Arc<dyn Inode> = self.inner.clone() as Arc<dyn Inode>;
        Ok(u32::from(inode.group()?))
    }
}

impl SharedMemMeta {
    pub fn new(shm_key: u32, is_anonymous: bool, size: usize, cpid: Pid) -> Self {
        let now = RealTimeCoarseClock::get().read_time();
        Self {
            is_anonymous,
            shm_deleted: false,
            shm_key,
            shm_size: size,
            shm_atime: Duration::ZERO,
            shm_dtime: Duration::ZERO,
            shm_ctime: now,
            shm_lpid: 0,
            shm_cpid: cpid,
        }
    }

    pub fn set_deleted(&mut self) {
        self.shm_deleted = true;
    }

    pub fn set_shm_atime(&mut self, time: Duration) {
        self.shm_atime = time;
    }

    pub fn set_shm_dtime(&mut self, time: Duration) {
        self.shm_dtime = time;
    }

    pub fn set_shm_ctime(&mut self, time: Duration) {
        self.shm_ctime = time;
    }

    pub fn set_shm_lpid(&mut self, pid: Pid) {
        self.shm_lpid = pid;
    }
}
