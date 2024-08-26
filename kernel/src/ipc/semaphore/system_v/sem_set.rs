// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::btree_map::BTreeMap, vec::Vec};
use core::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use aster_rights::ReadOp;
use id_alloc::IdAlloc;
use ostd::sync::{Mutex, RwMutex, RwMutexReadGuard, RwMutexWriteGuard};
use spin::Once;

use super::PermissionMode;
use crate::{
    ipc::{key_t, semaphore::system_v::sem::Semaphore, IpcPermission},
    prelude::*,
    process::Credentials,
    time::clocks::RealTimeCoarseClock,
};

// The following constant values are derived from the default values in Linux.

/// Maximum number of semaphore sets.
pub const SEMMNI: usize = 32000;
/// Maximum number of semaphores per semaphore ID.
pub const SEMMSL: usize = 32000;
/// Maximum number of seaphores in all semaphore sets.
pub const SEMMNS: usize = SEMMNI * SEMMSL;
/// Maximum number of operations for semop.
pub const SEMOPM: usize = 500;
/// MAximum semaphore value.
pub const SEMVMX: i32 = 32767;
/// Maximum value that can be recorded for semaphore adjustment (SEM_UNDO).
pub const SEMAEM: i32 = SEMVMX;

#[derive(Debug)]
pub struct SemaphoreSet {
    /// Number of semaphores in the set
    nsems: usize,
    /// Semaphores
    sems: Box<[Arc<Semaphore>]>,
    /// Semaphore permission
    permission: IpcPermission,
    /// Creation time or last modification via `semctl`
    sem_ctime: AtomicU64,
}

impl SemaphoreSet {
    pub fn nsems(&self) -> usize {
        self.nsems
    }

    pub fn get(&self, index: usize) -> Option<&Arc<Semaphore>> {
        self.sems.get(index)
    }

    pub fn permission(&self) -> &IpcPermission {
        &self.permission
    }

    pub fn sem_ctime(&self) -> Duration {
        Duration::from_secs(self.sem_ctime.load(Ordering::Relaxed))
    }

    pub fn update_ctime(&self) {
        self.sem_ctime.store(
            RealTimeCoarseClock::get().read_time().as_secs(),
            Ordering::Relaxed,
        );
    }

    fn new(key: key_t, nsems: usize, mode: u16, credentials: Credentials<ReadOp>) -> Result<Self> {
        debug_assert!(nsems <= SEMMSL);

        let mut sems = Vec::with_capacity(nsems);
        for _ in 0..nsems {
            sems.push(Arc::new(Semaphore::new(0)));
        }

        let permission =
            IpcPermission::new_sem_perm(key, credentials.euid(), credentials.egid(), mode);

        Ok(Self {
            nsems,
            sems: sems.into_boxed_slice(),
            permission,
            sem_ctime: AtomicU64::new(RealTimeCoarseClock::get().read_time().as_secs()),
        })
    }
}

impl Drop for SemaphoreSet {
    fn drop(&mut self) {
        for sem in self.sems.iter() {
            sem.removed();
        }

        ID_ALLOCATOR
            .get()
            .unwrap()
            .lock()
            .free(self.permission.key() as usize);
    }
}

pub fn create_sem_set_with_id(
    id: key_t,
    nsems: usize,
    mode: u16,
    credentials: Credentials<ReadOp>,
) -> Result<()> {
    debug_assert!(nsems <= SEMMSL);
    debug_assert!(id > 0);

    ID_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .alloc_specific(id as usize)
        .ok_or(Error::new(Errno::EEXIST))?;

    let mut sem_sets = SEMAPHORE_SETS.write();
    sem_sets.insert(id, SemaphoreSet::new(id, nsems, mode, credentials)?);

    Ok(())
}

/// Checks the semaphore. Return Ok if the semaphore exists and pass the check.
pub fn check_sem(id: key_t, nsems: Option<usize>, required_perm: PermissionMode) -> Result<()> {
    debug_assert!(id > 0);

    let sem_sets = SEMAPHORE_SETS.read();
    let sem_set = sem_sets.get(&id).ok_or(Error::new(Errno::ENOENT))?;

    if let Some(nsems) = nsems {
        debug_assert!(nsems <= SEMMSL);
        if nsems > sem_set.nsems() {
            return_errno!(Errno::EINVAL);
        }
    }

    if !required_perm.is_empty() {
        // TODO: Support permission check
        warn!("Semaphore doesn't support permission check now");
    }

    Ok(())
}

pub fn create_sem_set(nsems: usize, mode: u16, credentials: Credentials<ReadOp>) -> Result<key_t> {
    debug_assert!(nsems <= SEMMSL);

    let id = ID_ALLOCATOR
        .get()
        .unwrap()
        .lock()
        .alloc()
        .ok_or(Error::new(Errno::ENOSPC))? as i32;

    let mut sem_sets = SEMAPHORE_SETS.write();
    sem_sets.insert(id, SemaphoreSet::new(id, nsems, mode, credentials)?);

    Ok(id)
}

pub fn sem_sets<'a>() -> RwMutexReadGuard<'a, BTreeMap<key_t, SemaphoreSet>> {
    SEMAPHORE_SETS.read()
}

pub fn sem_sets_mut<'a>() -> RwMutexWriteGuard<'a, BTreeMap<key_t, SemaphoreSet>> {
    SEMAPHORE_SETS.write()
}

static ID_ALLOCATOR: Once<Mutex<IdAlloc>> = Once::new();

/// Semaphore sets in system
static SEMAPHORE_SETS: RwMutex<BTreeMap<key_t, SemaphoreSet>> = RwMutex::new(BTreeMap::new());

pub(super) fn init() {
    ID_ALLOCATOR.call_once(|| {
        let mut id_alloc = IdAlloc::with_capacity(SEMMNI + 1);
        // Remove the first index 0
        id_alloc.alloc();

        Mutex::new(id_alloc)
    });
}
