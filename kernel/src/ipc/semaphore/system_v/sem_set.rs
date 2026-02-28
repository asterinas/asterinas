// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use aster_rights::ReadOp;
use ostd::sync::PreemptDisabled;

use super::sem::{PendingOp, Status, update_pending_alter, wake_const_ops};
use crate::{
    ipc::{IpcPermission, key_t, semaphore::system_v::sem::Semaphore},
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
#[expect(dead_code)]
pub const SEMMNS: usize = SEMMNI * SEMMSL;
/// Maximum number of operations for semop.
pub const SEMOPM: usize = 500;
/// MAximum semaphore value.
pub const SEMVMX: i32 = 32767;
/// Maximum value that can be recorded for semaphore adjustment (SEM_UNDO).
#[expect(dead_code)]
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

// https://github.com/torvalds/linux/blob/master/include/uapi/asm-generic/ipcbuf.h
#[repr(C)]
#[padding_struct]
#[derive(Debug, Copy, Clone, Default, Pod)]
pub struct IpcPerm {
    key: u32,
    uid: u32,
    gid: u32,
    cuid: u32,
    cgid: u32,
    mode: u16,
    _pad1: u16,
    seq: u16,
    _pad2: u16,
    _unused1: u64,
    _unused2: u64,
}

// In Linux, most popular 64-bit architectures except x86_64 adopt the same
// layout of `semid_ds`.
// Reference: <https://elixir.bootlin.com/linux/v6.16.9/A/ident/semid64_ds>.
#[cfg(target_arch = "x86_64")]
#[repr(C)]
#[padding_struct]
#[derive(Debug, Copy, Clone, Default, Pod)]
pub struct SemidDs {
    sem_perm: IpcPerm,
    sem_otime: u64,
    _unused1: u64,
    sem_ctime: u64,
    _unused2: u64,
    sem_nsems: u64,
    _unused3: u64,
    _unused4: u64,
}

#[cfg(not(target_arch = "x86_64"))]
#[repr(C)]
#[padding_struct]
#[derive(Debug, Copy, Clone, Default, Pod)]
pub struct SemidDs {
    sem_perm: IpcPerm,
    sem_otime: u64,
    sem_ctime: u64,
    sem_nsems: u64,
    _unused3: u64,
    _unused4: u64,
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
    pub(super) fn field_mut(
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

    fn update_ctime(&self) {
        self.sem_ctime.store(
            RealTimeCoarseClock::get().read_time().as_secs(),
            Ordering::Relaxed,
        );
    }

    pub(super) fn update_otime(&self) {
        self.sem_otime.store(
            RealTimeCoarseClock::get().read_time().as_secs(),
            Ordering::Relaxed,
        );
    }

    pub(super) fn inner(&self) -> SpinLockGuard<'_, SemSetInner, PreemptDisabled> {
        self.inner.lock()
    }

    pub(crate) fn new(
        key: key_t,
        nsems: usize,
        mode: u16,
        credentials: Credentials<ReadOp>,
    ) -> Result<Self> {
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

    pub fn semid_ds(&self) -> SemidDs {
        let ipc_perm = IpcPerm {
            key: self.permission.key() as u32,
            uid: self.permission.uid().into(),
            gid: self.permission.gid().into(),
            cuid: self.permission.cuid().into(),
            cgid: self.permission.cguid().into(),
            mode: self.permission.mode(),
            ..IpcPerm::default()
        };

        SemidDs {
            sem_perm: ipc_perm,
            sem_otime: self.sem_otime.load(Ordering::Relaxed),
            sem_ctime: self.sem_ctime.load(Ordering::Relaxed),
            sem_nsems: self.nsems as u64,
            ..SemidDs::default()
        }
    }
}

impl Drop for SemaphoreSet {
    fn drop(&mut self) {
        let mut inner = self.inner();
        let pending_alter = &mut inner.pending_alter;
        for pending_alter in pending_alter.iter_mut() {
            pending_alter.set_status(Status::Removed);
            if let Some(waker) = pending_alter.waker() {
                waker.wake_up();
            }
        }
        pending_alter.clear();

        let pending_const = &mut inner.pending_const;
        for pending_const in pending_const.iter_mut() {
            pending_const.set_status(Status::Removed);
            if let Some(waker) = pending_const.waker() {
                waker.wake_up();
            }
        }
        pending_const.clear();
    }
}
