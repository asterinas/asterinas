// SPDX-License-Identifier: MPL-2.0

//! Shared memory.

use alloc::sync::Arc;
use core::{
    sync::atomic::{AtomicU32, Ordering},
    time::Duration,
};

use ostd::{
    mm::PAGE_SIZE,
    sync::{RwArc, SpinLock},
};

use crate::{
    fs::utils::{Inode, InodeMode},
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

/// Initializes the shared memory subsystem
pub fn init() {
    SHM_OBJ_MANAGER.call_once(|| RwArc::new(SharedMemManager::new()));
}

/// Shared memory object.
pub struct SharedMemObj {
    /// Shared memory files.
    inner: Arc<dyn Inode>,
    /// The reference count for the shared memory object.
    nlinks: AtomicU32,
    /// The metadata of the shared memory object.
    metadata: SpinLock<SharedMemMeta>,
}

impl Debug for SharedMemObj {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "SharedMemObj {{ shmid = {}, nlinks = {}, size = {}, metadata = {:?} }}",
            self.shmid(),
            self.nlinks(),
            self.size(),
            self.metadata.lock()
        )
    }
}

#[derive(Debug)]
pub struct SharedMemMeta {
    /// Indicates if the reference count is zero, deleting the object.
    shm_deleted: bool,

    /// Name of the shared memory key. None for anonymous objects.
    shm_key: Option<u32>,

    /// The shared memory ID (shmid).
    shmid: u64,

    /// Size of the shared memory object.
    shm_size: usize,

    /// Pid of last operator
    shm_lpid: Pid,
    /// Pid of creator
    shm_cpid: Pid,
}

impl SharedMemObj {
    /// Creates a new shared memory object.
    pub fn new(
        shm_file: Arc<dyn Inode>,
        key: Option<u32>,
        shmid: u64,
        size: usize,
        cpid: Pid,
    ) -> Self {
        Self {
            inner: shm_file,
            nlinks: AtomicU32::new(0),
            metadata: SpinLock::new(SharedMemMeta::new(key, shmid, size, cpid)),
        }
    }

    /// Gets the link count of the shared memory object.
    pub fn nlinks(&self) -> u32 {
        self.nlinks.load(Ordering::Relaxed)
    }

    /// Returns the key of the shared memory object.
    pub fn key(&self) -> Option<u32> {
        self.metadata.lock().shm_key
    }

    /// Returns whether the shared memory object is anonymous.
    pub fn is_anonymous(&self) -> bool {
        self.metadata.lock().shm_key.is_none()
    }

    /// Increases the reference count of the shared memory object.
    pub fn inc_nlinks(&self) -> u32 {
        self.nlinks.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Sets the shared memory object as attached and updates the related
    /// metadata.
    pub fn set_attached(&self, lpid: Pid) {
        let now = RealTimeCoarseClock::get().read_time();

        // Set atime using the encapsulated function
        self.set_shm_atime(now);

        let mut meta = self.metadata.lock();
        meta.set_shm_lpid(lpid);
    }

    /// Decreases the reference count of the shared memory object.
    fn dec_nlinks(&self) -> u32 {
        self.nlinks.fetch_sub(1, Ordering::Relaxed) - 1
    }

    /// Decreases the reference count of the shared memory object.
    pub fn set_detached(&self, lpid: Pid) -> u32 {
        let now = RealTimeCoarseClock::get().read_time();

        self.set_shm_dtime(now);
        self.metadata.lock().set_shm_lpid(lpid);
        self.dec_nlinks()
    }

    /// Returns the shared memory id.
    pub fn shmid(&self) -> u64 {
        self.metadata.lock().shmid
    }

    /// Sets the shared memory last attach time (atime) mapped to inode's atime.
    pub fn set_shm_atime(&self, time: Duration) {
        self.inner.set_atime(time)
    }

    /// Gets the shared memory access time (atime) from inode's atime.
    pub fn shm_atime(&self) -> Duration {
        self.inner.atime()
    }

    /// Sets the shared memory last detach time (dtime) mapped to inode's mtime.
    pub fn set_shm_dtime(&self, time: Duration) {
        self.inner.set_mtime(time)
    }

    /// Gets the shared memory detach time (dtime) from inode's mtime.
    pub fn shm_dtime(&self) -> Duration {
        self.inner.mtime()
    }

    /// Sets the shared memory last change time (ctime) mapped to inode's ctime.
    pub fn set_shm_ctime(&self, time: Duration) {
        self.inner.set_ctime(time)
    }

    /// Gets the shared memory change time (ctime) from inode's ctime.
    pub fn shm_ctime(&self) -> Duration {
        self.inner.ctime()
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

    /// Returns the mode of the shared memory object.
    pub fn mode(&self) -> Result<InodeMode> {
        self.inner.mode()
    }

    /// Sets the mode of the shared memory object.
    pub fn set_mode(&self, mode: InodeMode) -> Result<()> {
        self.inner.set_mode(mode)
    }

    /// Returns the VMO of the shared memory object.
    pub fn vmo(&self) -> Result<Vmo> {
        let vmo = self
            .inner
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
        self.inner.set_owner(Uid::from(uid))?;
        self.inner.set_group(Gid::from(gid))?;

        let now = RealTimeCoarseClock::get().read_time();
        self.set_shm_ctime(now);

        Ok(())
    }

    /// Returns the attributes of the shared memory object.
    pub fn get_attributes(&self) -> Result<ShmidDs> {
        let meta = self.metadata.lock();

        Ok(ShmidDs {
            shm_perm: IpcPerm {
                key: meta.shm_key.unwrap_or(0) as i32,
                uid: u32::from(self.inner.owner()?),
                gid: u32::from(self.inner.group()?),
                cuid: meta.shm_cpid,
                cgid: u32::from(self.inner.group()?),
                mode: self.mode()?.bits(),
                seq: 0,
                _pad2: 0,
                _glibc_reserved1: 0,
                _glibc_reserved2: 0,
            },
            shm_segsz: meta.shm_size,
            shm_atime: self.shm_atime().as_secs() as i64,
            shm_dtime: self.shm_dtime().as_secs() as i64,
            shm_ctime: self.shm_ctime().as_secs() as i64,
            shm_cpid: meta.shm_cpid as i32,
            shm_lpid: meta.shm_lpid as i32,
            shm_nattch: self.nlinks() as u64,
            _glibc_reserved5: 0,
            _glibc_reserved6: 0,
        })
    }

    /// Returns the uid of the shared memory object.
    pub fn uid(&self) -> Result<u32> {
        Ok(u32::from(self.inner.owner()?))
    }

    /// Returns the gid of the shared memory object.
    pub fn gid(&self) -> Result<u32> {
        Ok(u32::from(self.inner.group()?))
    }
}

impl SharedMemMeta {
    fn new(shm_key: Option<u32>, shmid: u64, size: usize, cpid: Pid) -> Self {
        Self {
            shm_deleted: false,
            shm_key,
            shmid,
            shm_size: size,
            shm_lpid: 0,
            shm_cpid: cpid,
        }
    }

    /// Sets the shared memory object as deleted.
    ///
    /// It is **inevitable** to mark the shared memory object as deleted.
    /// Attaching to the shared memory object that will be deleted is still
    /// permitted, but once the deleted flag is set, it cannot be reversed.
    fn set_deleted(&mut self) {
        self.shm_deleted = true;
    }

    fn set_shm_lpid(&mut self, pid: Pid) {
        self.shm_lpid = pid;
    }
}
