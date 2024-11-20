// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::btree_map::BTreeMap, vec::Vec};
use core::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use aster_rights::ReadOp;
use id_alloc::IdAlloc;
use ostd::sync::{PreemptDisabled, RwLockReadGuard, RwLockWriteGuard};
use spin::Once;

use super::{
    sem::{update_pending_alter, wake_const_ops, PendingOp, Status},
    PermissionMode,
};
use crate::{
    ipc::{key_t, semaphore::system_v::sem::Semaphore, IpcPermission},
    prelude::*,
    process::{Credentials, Pid},
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
    /// Inner
    inner: SpinLock<SemSetInner>,
    /// Semaphore permission
    permission: IpcPermission,
    /// Creation time or last modification via `semctl`
    sem_ctime: AtomicU64,
    /// Last semop time.
    sem_otime: AtomicU64,
}

#[derive(Debug)]
pub(super) struct SemSetInner {
    /// Semaphores
    pub(super) sems: Box<[Semaphore]>,
    /// Pending alter operations.
    pub(super) pending_alter: LinkedList<PendingOp>,
    /// Pending zeros operations.
    pub(super) pending_const: LinkedList<PendingOp>,
}

impl SemSetInner {
    pub fn field_mut(
        &mut self,
    ) -> (
        &mut Box<[Semaphore]>,
        &mut LinkedList<PendingOp>,
        &mut LinkedList<PendingOp>,
    ) {
        (
            &mut self.sems,
            &mut self.pending_alter,
            &mut self.pending_const,
        )
    }
}

impl SemaphoreSet {
    pub fn pending_const_count(&self, sem_num: u16) -> usize {
        let inner = self.inner.lock();
        let pending_const = &inner.pending_const;
        let mut count = 0;
        for i in pending_const.iter() {
            for sem_buf in i.sops_iter() {
                if sem_buf.sem_num() == sem_num {
                    count += 1;
                }
            }
        }
        count
    }

    pub fn pending_alter_count(&self, sem_num: u16) -> usize {
        let inner = self.inner.lock();
        let pending_alter = &inner.pending_alter;
        let mut count = 0;
        for i in pending_alter.iter() {
            for sem_buf in i.sops_iter() {
                if sem_buf.sem_num() == sem_num {
                    count += 1;
                }
            }
        }
        count
    }

    pub fn nsems(&self) -> usize {
        self.nsems
    }

    pub fn setval(&self, sem_num: usize, val: i32, pid: Pid) -> Result<()> {
        if !(0..SEMVMX).contains(&val) {
            return_errno!(Errno::ERANGE);
        }

        let mut inner = self.inner();
        let (sems, pending_alter, pending_const) = inner.field_mut();
        let sem = sems.get_mut(sem_num).ok_or(Error::new(Errno::EINVAL))?;

        sem.set_val(val);
        sem.set_latest_modified_pid(pid);

        let mut wake_queue = LinkedList::new();
        if val == 0 {
            wake_const_ops(sems, pending_const, &mut wake_queue);
        } else {
            update_pending_alter(sems, pending_alter, pending_const, &mut wake_queue);
        }

        for wake_op in wake_queue {
            wake_op.set_status(Status::Normal);
            if let Some(waker) = wake_op.waker() {
                waker.wake_up();
            }
        }

        self.update_ctime();
        Ok(())
    }

    pub fn get<T>(&self, sem_num: usize, func: &dyn Fn(&Semaphore) -> T) -> Result<T> {
        let inner = self.inner();
        Ok(func(
            inner.sems.get(sem_num).ok_or(Error::new(Errno::EINVAL))?,
        ))
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

    pub fn update_otime(&self) {
        self.sem_otime.store(
            RealTimeCoarseClock::get().read_time().as_secs(),
            Ordering::Relaxed,
        );
    }

    pub(super) fn inner(&self) -> SpinLockGuard<SemSetInner, PreemptDisabled> {
        self.inner.lock()
    }

    fn new(key: key_t, nsems: usize, mode: u16, credentials: Credentials<ReadOp>) -> Result<Self> {
        debug_assert!(nsems <= SEMMSL);

        let mut sems = Vec::with_capacity(nsems);
        for _ in 0..nsems {
            sems.push(Semaphore::new(0));
        }

        let permission =
            IpcPermission::new_sem_perm(key, credentials.euid(), credentials.egid(), mode);

        Ok(Self {
            nsems,
            permission,
            sem_ctime: AtomicU64::new(RealTimeCoarseClock::get().read_time().as_secs()),
            sem_otime: AtomicU64::new(0),
            inner: SpinLock::new(SemSetInner {
                sems: sems.into_boxed_slice(),
                pending_alter: LinkedList::new(),
                pending_const: LinkedList::new(),
            }),
        })
    }
}

impl Drop for SemaphoreSet {
    fn drop(&mut self) {
        let mut inner = self.inner();
        let pending_alter = &mut inner.pending_alter;
        for pending_alter in pending_alter.iter_mut() {
            pending_alter.set_status(Status::Removed);
            if let Some(ref waker) = pending_alter.waker() {
                waker.wake_up();
            }
        }
        pending_alter.clear();

        let pending_const = &mut inner.pending_const;
        for pending_const in pending_const.iter_mut() {
            pending_const.set_status(Status::Removed);
            if let Some(ref waker) = pending_const.waker() {
                waker.wake_up();
            }
        }
        pending_const.clear();

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
    if id as usize > SEMMNI {
        return_errno_with_message!(Errno::ENOENT, "id larger than SEMMNI");
    }

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

pub fn sem_sets<'a>() -> RwLockReadGuard<'a, BTreeMap<key_t, SemaphoreSet>, PreemptDisabled> {
    SEMAPHORE_SETS.read()
}

pub fn sem_sets_mut<'a>() -> RwLockWriteGuard<'a, BTreeMap<key_t, SemaphoreSet>, PreemptDisabled> {
    SEMAPHORE_SETS.write()
}

static ID_ALLOCATOR: Once<SpinLock<IdAlloc>> = Once::new();

/// Semaphore sets in system
static SEMAPHORE_SETS: RwLock<BTreeMap<key_t, SemaphoreSet>> = RwLock::new(BTreeMap::new());

pub(super) fn init() {
    ID_ALLOCATOR.call_once(|| {
        let mut id_alloc = IdAlloc::with_capacity(SEMMNI + 1);
        // Remove the first index 0
        id_alloc.alloc();

        SpinLock::new(id_alloc)
    });
}
