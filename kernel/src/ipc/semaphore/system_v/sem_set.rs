// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use aster_rights::ReadOp;

use super::sem::{
    PendingBlocker, PendingOp, Semaphore, Status, update_pending_alter, wake_const_ops,
};
use crate::{
    ipc::{IpcKey, IpcPermission},
    prelude::*,
    process::{Credentials, Pid},
    time::clocks::RealTimeCoarseClock,
};

// The following constant values are derived from the default values in Linux.

/// Maximum number of semaphore sets.
pub const SEMMNI: usize = 32000;
/// Maximum number of semaphores per semaphore ID.
pub const SEMMSL: usize = 32000;
/// Maximum number of semaphores in all semaphore sets.
#[expect(dead_code)]
pub const SEMMNS: usize = SEMMNI * SEMMSL;
/// Maximum number of operations for semop.
pub const SEMOPM: usize = 500;
/// Maximum semaphore value.
pub const SEMVMX: i32 = 32767;
/// Maximum value that can be recorded for semaphore adjustment (SEM_UNDO).
#[expect(dead_code)]
pub const SEMAEM: i32 = SEMVMX;

#[derive(Debug)]
pub struct SemaphoreSet {
    /// Number of semaphores in the set
    num_sems: usize,
    /// Inner
    inner: Mutex<SemSetInner>,
    /// Semaphore permission
    permission: IpcPermission,
    /// Creation time or last modification via `semctl`
    sem_ctime: AtomicU64,
    /// Last `semop` time
    sem_otime: AtomicU64,
}

// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/asm-generic/ipcbuf.h#L22>.
#[padding_struct]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct IpcPerm {
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
#[padding_struct]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
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
#[padding_struct]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
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
    /// Counts the number of pending operations waiting for the semaphore at `sem_num` to become
    /// zero.
    pub fn pending_zero_count(&self, sem_num: usize) -> Result<usize> {
        if sem_num >= self.num_sems {
            return_errno_with_message!(Errno::EINVAL, "the semaphore number is out of bounds");
        }

        let inner = self.inner.lock();
        let count_const = inner
            .pending_const
            .iter()
            .filter(|op| op.blocker(&inner.sems) == Some(PendingBlocker::Zero(sem_num)))
            .count();
        let count_alter = inner
            .pending_alter
            .iter()
            .filter(|op| op.blocker(&inner.sems) == Some(PendingBlocker::Zero(sem_num)))
            .count();
        Ok(count_const + count_alter)
    }

    /// Counts the number of pending operations waiting for the semaphore at `sem_num` to be able to
    /// decrease by a certain amount.
    pub fn pending_decrease_count(&self, sem_num: usize) -> Result<usize> {
        if sem_num >= self.num_sems {
            return_errno_with_message!(Errno::EINVAL, "the semaphore number is out of bounds");
        }

        let inner = self.inner.lock();
        let count = inner
            .pending_alter
            .iter()
            .filter(|op| op.blocker(&inner.sems) == Some(PendingBlocker::Decrease(sem_num)))
            .count();
        Ok(count)
    }

    pub fn num_sems(&self) -> usize {
        self.num_sems
    }

    pub fn setval(&self, sem_num: usize, val: i32, pid: Pid) -> Result<()> {
        if !(0..=SEMVMX).contains(&val) {
            return_errno_with_message!(Errno::ERANGE, "the semaphore value exceeds SEMVMX");
        }

        let mut inner = self.inner();
        let (sems, pending_alter, pending_const) = inner.field_mut();
        let Some(sem) = sems.get_mut(sem_num) else {
            return_errno_with_message!(Errno::EINVAL, "the semaphore number is out of bounds");
        };

        sem.set_val(val);
        sem.set_latest_modified_pid(pid);

        let mut wake_queue = LinkedList::new();
        if val == 0 {
            wake_const_ops(sems, pending_const, &mut wake_queue);
        }
        update_pending_alter(sems, pending_alter, pending_const, &mut wake_queue);

        for wake_op in wake_queue {
            wake_op.set_status(Status::Normal);
            if let Some(waker) = wake_op.waker() {
                waker.wake_up();
            }
        }

        self.update_ctime();
        Ok(())
    }

    pub fn get<T>(&self, sem_num: usize, func: fn(&Semaphore) -> T) -> Result<T> {
        let inner = self.inner();
        let Some(sem) = inner.sems.get(sem_num) else {
            return_errno_with_message!(Errno::EINVAL, "the semaphore number is out of bounds");
        };

        let result = func(sem);
        Ok(result)
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

    pub(super) fn inner(&self) -> MutexGuard<'_, SemSetInner> {
        self.inner.lock()
    }

    pub(in crate::ipc) fn new(
        key: IpcKey,
        num_sems: usize,
        mode: u16,
        credentials: &Credentials<ReadOp>,
    ) -> Result<Self> {
        if num_sems == 0 {
            return_errno_with_message!(Errno::EINVAL, "the number of semaphores is zero")
        }
        if num_sems > SEMMSL {
            return_errno_with_message!(Errno::EINVAL, "the number of semaphores exceeds SEMMSL");
        }

        let mut sems = Vec::with_capacity(num_sems);
        for _ in 0..num_sems {
            sems.push(Semaphore::new(0));
        }

        let permission =
            IpcPermission::new_sem_perm(key, credentials.euid(), credentials.egid(), mode);

        Ok(Self {
            num_sems,
            permission,
            sem_ctime: AtomicU64::new(RealTimeCoarseClock::get().read_time().as_secs()),
            sem_otime: AtomicU64::new(0),
            inner: Mutex::new(SemSetInner {
                sems: sems.into_boxed_slice(),
                pending_alter: LinkedList::new(),
                pending_const: LinkedList::new(),
            }),
        })
    }

    pub fn semid_ds(&self) -> SemidDs {
        let ipc_perm = IpcPerm {
            key: self.permission.key().cast_unsigned(),
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
            sem_nsems: self.num_sems as u64,
            ..SemidDs::default()
        }
    }
}

impl Drop for SemaphoreSet {
    fn drop(&mut self) {
        let inner = self.inner.get_mut();

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
