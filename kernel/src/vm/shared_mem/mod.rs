// SPDX-License-Identifier: MPL-2.0

//! Shared memory.

use alloc::sync::Arc;
use core::{
    sync::atomic::{AtomicU32, AtomicU64, Ordering},
    time::Duration,
};

use ostd::{mm::PAGE_SIZE, sync::RwLock};

use crate::{
    fs::utils::InodeMode,
    prelude::*,
    process::{Gid, Pid, Uid},
    time::clocks::RealTimeCoarseClock,
    vm::vmo::Vmo,
};

mod ipc_types;
mod manager;

pub use ipc_types::{IpcPerm, ShmidDs};
pub use manager::{SHM_OBJ_MANAGER, SharedMemManager};

pub const SHMMIN: usize = 1; // Minimum shared segment size in bytes
pub const SHMMAX: usize = usize::MAX - (1 << 24); // Maximum shared segment size in bytes
pub const SHMLBA: usize = PAGE_SIZE; // Shared memory segment alignment

/// Identifier of a shared memory attachment instance within a single VMAR.
///
/// A single System V shared memory segment (`shmid`) may be attached multiple
/// times within the same process. Each `shmat()` creates a distinct attachment
/// instance, identified by `shmat_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AttachedShm {
    pub shmid: u64,
    pub shmat_id: u64,
}

/// Initializes the shared memory subsystem
pub fn init() {
    SHM_OBJ_MANAGER.call_once(|| RwLock::new(SharedMemManager::new()));
}

/// Shared memory object.
pub struct SharedMemObj {
    /// The VMO backing this shared memory object.
    vmo: Arc<Vmo>,
    /// The number of attachments.
    attach_count: AtomicU32,
    /// Counter for allocating `shmat_id`.
    shmat_id_counter: AtomicU64,
    /// Metadata of the shared memory object.
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

    /// Owner UID.
    uid: Uid,
    /// Owner GID.
    gid: Gid,
    /// Permission bits.
    mode: InodeMode,

    /// Access time.
    shm_atime: Duration,
    /// Detach time.
    shm_dtime: Duration,
    /// Change time.
    shm_ctime: Duration,
}

impl SharedMemObj {
    /// Creates a new shared memory object.
    fn new(
        vmo: Arc<Vmo>,
        key: Option<u32>,
        shmid: u64,
        size: usize,
        cpid: Pid,
        mode: InodeMode,
    ) -> Self {
        let uid = Uid::new_root();
        let gid = Gid::new_root();
        Self {
            vmo,
            attach_count: AtomicU32::new(0),
            shmat_id_counter: AtomicU64::new(0),
            metadata: SpinLock::new(SharedMemMeta::new(key, shmid, size, cpid, uid, gid, mode)),
        }
    }

    /// Gets the link count of the shared memory object.
    fn nlinks(&self) -> u32 {
        self.attach_count.load(Ordering::Acquire)
    }

    /// Returns the key of the shared memory object.
    pub fn key(&self) -> Option<u32> {
        self.metadata.lock().shm_key
    }

    /// Increments the attachment count for an already-attached mapping.
    ///
    /// This is used when an existing `shmat()` mapping is duplicated (e.g., by
    /// splitting VMAs during `munmap`, or by inheriting mappings during
    /// `fork()`), where we want to preserve the same [`AttachedShm`]
    /// identifier while still bumping the attachment count.
    pub fn inc_nattch_for_mapping(&self, lpid: Pid) -> u32 {
        let now = RealTimeCoarseClock::get().read_time();
        self.set_shm_atime(now);
        self.metadata.lock().set_shm_lpid(lpid);
        self.attach_count.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// Marks this segment as attached and returns an identifier for this
    /// attachment instance.
    pub fn set_attached(&self, lpid: Pid) -> AttachedShm {
        self.inc_nattch_for_mapping(lpid);

        let shmat_id = self.shmat_id_counter.fetch_add(1, Ordering::Relaxed);
        AttachedShm {
            shmid: self.shmid(),
            shmat_id,
        }
    }

    /// Marks this segment as detached and returns the new attachment count.
    pub fn set_detached(&self, lpid: Pid) -> u32 {
        let now = RealTimeCoarseClock::get().read_time();

        self.set_shm_dtime(now);
        self.metadata.lock().set_shm_lpid(lpid);
        self.attach_count.fetch_sub(1, Ordering::AcqRel) - 1
    }

    /// Returns the shared memory id.
    pub fn shmid(&self) -> u64 {
        self.metadata.lock().shmid
    }

    /// Sets the shared memory last attach time (atime) mapped to inode's atime.
    fn set_shm_atime(&self, time: Duration) {
        self.metadata.lock().shm_atime = time;
    }

    /// Sets the shared memory last detach time (dtime) mapped to inode's mtime.
    fn set_shm_dtime(&self, time: Duration) {
        self.metadata.lock().shm_dtime = time;
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
        Ok(self.metadata.lock().mode)
    }

    /// Returns the VMO of the shared memory object.
    pub fn vmo(&self) -> Arc<Vmo> {
        self.vmo.clone()
    }

    /// Sets the attributes of the shared memory object.
    pub fn set_attributes(&self, mode: InodeMode, uid: u32, gid: u32) -> Result<()> {
        let mut meta = self.metadata.lock();
        meta.mode = mode;
        meta.uid = Uid::from(uid);
        meta.gid = Gid::from(gid);

        let now = RealTimeCoarseClock::get().read_time();
        meta.shm_ctime = now;

        Ok(())
    }

    /// Returns the attributes of the shared memory object.
    pub fn get_attributes(&self) -> Result<ShmidDs> {
        let meta = self.metadata.lock();

        Ok(ShmidDs {
            shm_perm: IpcPerm {
                key: meta.shm_key.unwrap_or(0) as i32,
                uid: u32::from(meta.uid),
                gid: u32::from(meta.gid),
                cuid: meta.shm_cpid,
                cgid: u32::from(meta.gid),
                mode: meta.mode.bits(),
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
            shm_nattch: self.nlinks() as u64,
            _glibc_reserved5: 0,
            _glibc_reserved6: 0,
        })
    }

    /// Returns the uid of the shared memory object.
    pub fn uid(&self) -> Result<u32> {
        Ok(u32::from(self.metadata.lock().uid))
    }

    /// Returns the gid of the shared memory object.
    pub fn gid(&self) -> Result<u32> {
        Ok(u32::from(self.metadata.lock().gid))
    }
}

impl SharedMemMeta {
    fn new(
        shm_key: Option<u32>,
        shmid: u64,
        size: usize,
        cpid: Pid,
        uid: Uid,
        gid: Gid,
        mode: InodeMode,
    ) -> Self {
        let now = RealTimeCoarseClock::get().read_time();
        Self {
            shm_deleted: false,
            shm_key,
            shmid,
            shm_size: size,
            shm_lpid: 0,
            shm_cpid: cpid,
            uid,
            gid,
            mode,
            shm_atime: now,
            shm_dtime: now,
            shm_ctime: now,
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
