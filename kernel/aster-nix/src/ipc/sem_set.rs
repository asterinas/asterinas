// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::btree_map::BTreeMap, vec::Vec};
use core::time::Duration;

use id_alloc::IdAlloc;
use ostd::sync::{Mutex, RwMutex, RwMutexReadGuard, RwMutexWriteGuard};
use spin::Once;

use super::{key_t, IpcPermission};
use crate::{
    ipc::sem::Semaphore,
    prelude::*,
    process::{Gid, Uid},
    time::SystemTime,
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
/// Maximum value that can be recored for semaphore adjustment (SEM_UNDO).
pub const SEMAEM: i32 = SEMVMX;

static ID_ALLOCATOR: Once<Mutex<IdAlloc>> = Once::new();

/// Semaphore sets in system
static SEMAPHORE_SETS: RwMutex<BTreeMap<key_t, SemaphoreSet>> = RwMutex::new(BTreeMap::new());

#[derive(Debug)]
pub struct SemaphoreSet {
    /// Number of semaphores in the set
    nsems: usize,
    /// Semaphores
    sems: Vec<Semaphore>,
    /// Semaphore permission
    permission: IpcPermission,
    /// Inner
    inner: Mutex<Inner>,
}

#[derive(Debug)]
struct Inner {
    /// Last semop time
    sem_otime: Duration,
    /// Creation time or last modification via `semctl`
    sem_ctime: Duration,
}

impl SemaphoreSet {
    pub fn nsems(&self) -> usize {
        self.nsems
    }

    pub fn get(&self, index: usize) -> Option<&Semaphore> {
        self.sems.get(index)
    }

    pub fn permission(&self) -> &IpcPermission {
        &self.permission
    }

    pub fn update_ctime(&self) {
        let now = SystemTime::now()
            .duration_since(&SystemTime::UNIX_EPOCH)
            .unwrap();

        self.inner.lock().sem_ctime = now;
    }

    pub fn update_otime(&self) {
        let now = SystemTime::now()
            .duration_since(&SystemTime::UNIX_EPOCH)
            .unwrap();

        self.inner.lock().sem_otime = now;
    }

    fn new(key: key_t, nsems: usize, mode: u16) -> Result<Self> {
        debug_assert!(nsems <= SEMMSL);

        let mut sems = Vec::with_capacity(nsems);
        for _ in 0..nsems {
            sems.push(Semaphore::new(0));
        }

        // FIXME: Use correct uid and gid here.
        let permission = IpcPermission::new_sem_perm(key, Uid::new_root(), Gid::new_root(), mode);
        let now = SystemTime::now()
            .duration_since(&SystemTime::UNIX_EPOCH)
            .unwrap();

        Ok(Self {
            nsems,
            sems,
            permission,
            inner: Mutex::new(Inner {
                sem_otime: Duration::from_secs(0),
                sem_ctime: now,
            }),
        })
    }
}

pub fn sem_sets<'a>() -> RwMutexReadGuard<'a, BTreeMap<key_t, SemaphoreSet>> {
    SEMAPHORE_SETS.read()
}

pub fn sem_sets_mut<'a>() -> RwMutexWriteGuard<'a, BTreeMap<key_t, SemaphoreSet>> {
    SEMAPHORE_SETS.write()
}

pub(super) fn init() {
    ID_ALLOCATOR.call_once(|| {
        let mut id_alloc = IdAlloc::with_capacity(SEMMNI + 1);
        // Remove the first index 0
        id_alloc.alloc();

        Mutex::new(id_alloc)
    });
}
