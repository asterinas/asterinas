// SPDX-License-Identifier: MPL-2.0

use core::{
    sync::atomic::{AtomicU16, AtomicU64, Ordering},
    time::Duration,
};

use ostd::sync::{Mutex, Waiter, Waker};

use super::sem_set::SEMVMX;
use crate::{
    ipc::{key_t, semaphore::system_v::sem_set::sem_sets, IpcFlags},
    prelude::*,
    process::{Pid, Process},
    time::{
        clocks::{RealTimeCoarseClock, JIFFIES_TIMER_MANAGER},
        timer::Timeout,
    },
};

#[derive(Clone, Copy, Debug, Pod)]
#[repr(C)]
pub struct SemBuf {
    sem_num: u16,
    sem_op: i16,
    sem_flags: i16,
}

impl SemBuf {
    pub fn sem_num(&self) -> u16 {
        self.sem_num
    }

    pub fn sem_op(&self) -> i16 {
        self.sem_op
    }

    pub fn sem_flags(&self) -> i16 {
        self.sem_flags
    }
}

#[repr(u16)]
#[derive(Debug, TryFromInt, Clone, Copy)]
enum Status {
    Normal = 0,
    Pending = 1,
    Removed = 2,
}

struct AtomicStatus(AtomicU16);

impl AtomicStatus {
    fn new(status: Status) -> Self {
        Self(AtomicU16::new(status as u16))
    }

    fn status(&self) -> Status {
        Status::try_from(self.0.load(Ordering::Relaxed)).unwrap()
    }

    fn set_status(&self, status: Status) {
        self.0.store(status as u16, Ordering::Relaxed);
    }
}

struct PendingOp {
    sem_buf: SemBuf,
    status: Arc<AtomicStatus>,
    waker: Arc<Waker>,
    pid: Pid,
    process: Weak<Process>,
}

impl Debug for PendingOp {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PendingOp")
            .field("sem_buf", &self.sem_buf)
            .field("status", &(self.status.status()))
            .field("pid", &self.pid)
            .finish()
    }
}

#[derive(Debug)]
pub struct Semaphore {
    val: Mutex<i32>,
    /// PID of the process that last modified semaphore.
    /// - through semop with op != 0
    /// - through semctl with SETVAL and SETALL
    /// - through SEM_UNDO when task exit
    latest_modified_pid: RwMutex<Pid>,
    /// Pending alter operations. For each pending operation, it has `sem_op < 0`.
    pending_alters: Mutex<LinkedList<Box<PendingOp>>>,
    /// Pending zeros operations. For each pending operation, it has `sem_op = 0`.
    pending_const: Mutex<LinkedList<Box<PendingOp>>>,
    /// Last semop time.
    sem_otime: AtomicU64,
}

impl Semaphore {
    pub fn set_val(&self, val: i32, current_pid: Pid) -> Result<()> {
        if !(0..SEMVMX).contains(&val) {
            return_errno!(Errno::ERANGE);
        }

        let mut current_val = self.val.lock();
        *current_val = val;
        *self.latest_modified_pid.write() = current_pid;

        self.update_pending_ops(current_val);
        Ok(())
    }

    pub fn val(&self) -> i32 {
        *self.val.lock()
    }

    pub fn last_modified_pid(&self) -> Pid {
        *self.latest_modified_pid.read()
    }

    pub fn sem_otime(&self) -> Duration {
        Duration::from_secs(self.sem_otime.load(Ordering::Relaxed))
    }

    pub fn pending_zero_count(&self) -> usize {
        self.pending_const.lock().len()
    }

    pub fn pending_alter_count(&self) -> usize {
        self.pending_alters.lock().len()
    }

    /// Notifies the semaphore that the semaphore sets it belongs to have been removed.
    pub(super) fn removed(&self) {
        let mut pending_alters = self.pending_alters.lock();
        for pending_alter in pending_alters.iter_mut() {
            pending_alter.status.set_status(Status::Removed);
            pending_alter.waker.wake_up();
        }
        pending_alters.clear();

        let mut pending_const = self.pending_const.lock();
        for pending_const in pending_const.iter_mut() {
            pending_const.status.set_status(Status::Removed);
            pending_const.waker.wake_up();
        }
        pending_const.clear();
    }

    pub(super) fn new(val: i32) -> Self {
        Self {
            val: Mutex::new(val),
            latest_modified_pid: RwMutex::new(current!().pid()),
            pending_alters: Mutex::new(LinkedList::new()),
            pending_const: Mutex::new(LinkedList::new()),
            sem_otime: AtomicU64::new(0),
        }
    }

    fn update_otime(&self) {
        self.sem_otime.store(
            RealTimeCoarseClock::get().read_time().as_secs(),
            Ordering::Relaxed,
        );
    }

    fn sem_op(&self, sem_buf: &SemBuf, timeout: Option<Duration>, ctx: &Context) -> Result<()> {
        let mut val = self.val.lock();
        let sem_op = sem_buf.sem_op;
        let current_pid = ctx.process.pid();

        let flags = IpcFlags::from_bits(sem_buf.sem_flags as u32).unwrap();
        if flags.contains(IpcFlags::SEM_UNDO) {
            todo!()
        }

        // Operate val
        let positive_condition = sem_op.is_positive();
        let negative_condition = sem_op.is_negative() && sem_op.abs() as i32 <= *val;
        let zero_condition = sem_op == 0 && *val == 0;

        if positive_condition || negative_condition {
            let new_val = val
                .checked_add(i32::from(sem_op))
                .ok_or(Error::new(Errno::ERANGE))?;
            if new_val > SEMVMX {
                return_errno!(Errno::ERANGE);
            }

            *val = new_val;
            *self.latest_modified_pid.write() = current_pid;
            self.update_otime();

            self.update_pending_ops(val);
            return Ok(());
        } else if zero_condition {
            return Ok(());
        }
        drop(val);

        // Need to wait for the semaphore
        if flags.contains(IpcFlags::IPC_NOWAIT) {
            return_errno!(Errno::EAGAIN);
        }

        // Add current to pending list
        let (waiter, waker) = Waiter::new_pair();
        let status = Arc::new(AtomicStatus::new(Status::Pending));
        let pending_op = Box::new(PendingOp {
            sem_buf: *sem_buf,
            status: status.clone(),
            waker: waker.clone(),
            process: ctx.posix_thread.weak_process(),
            pid: current_pid,
        });
        if sem_op == 0 {
            self.pending_const.lock().push_back(pending_op);
        } else {
            self.pending_alters.lock().push_back(pending_op);
        }

        // Wait
        if let Some(timeout) = timeout {
            let jiffies_timer = JIFFIES_TIMER_MANAGER.get().unwrap().create_timer(move || {
                waker.wake_up();
            });
            jiffies_timer.set_timeout(Timeout::After(timeout));
        }
        waiter.wait();

        // Check status and return
        match status.status() {
            Status::Normal => Ok(()),
            Status::Removed => Err(Error::new(Errno::EIDRM)),
            Status::Pending => {
                let mut pending_ops = if sem_op == 0 {
                    self.pending_const.lock()
                } else {
                    self.pending_alters.lock()
                };
                pending_ops.retain(|op| op.pid != current_pid);
                Err(Error::new(Errno::EAGAIN))
            }
        }
    }

    /// Updates pending ops after the val changed.
    fn update_pending_ops(&self, mut val: MutexGuard<i32>) {
        debug_assert!(*val >= 0);
        trace!("Updating pending ops, semaphore before: {:?}", *val);

        // Two steps:
        // 1. Remove the pending_alters with `sem_op < 0` if it can.
        // 2. If val is equal to 0, then clear pending_const

        // Step one:
        let mut pending_alters = self.pending_alters.lock();
        pending_alters.retain_mut(|op| {
            if *val == 0 {
                return true;
            }
            // Check if the process alive.
            if op.process.upgrade().is_none() {
                return false;
            }
            debug_assert!(op.sem_buf.sem_op < 0);

            if op.sem_buf.sem_op.abs() as i32 <= *val {
                trace!(
                    "Found removable pending op, op: {:?}, pid:{:?}",
                    op.sem_buf.sem_op,
                    op.pid
                );

                *val += i32::from(op.sem_buf.sem_op);
                *self.latest_modified_pid.write() = op.pid;
                self.update_otime();
                op.status.set_status(Status::Normal);
                op.waker.wake_up();
                false
            } else {
                true
            }
        });

        // Step two:
        if *val == 0 {
            let mut pending_const = self.pending_const.lock();
            pending_const.iter().for_each(|op| {
                op.status.set_status(Status::Normal);
                if op.process.upgrade().is_some() {
                    trace!("Found removable pending op, op: 0, pid:{:?}", op.pid);
                    op.waker.wake_up();
                }
            });
            pending_const.clear();
        }
        trace!("Updated pending ops, semaphore after: {:?}", *val);
    }
}

pub fn sem_op(
    sem_id: key_t,
    sem_buf: &SemBuf,
    timeout: Option<Duration>,
    ctx: &Context,
) -> Result<()> {
    debug_assert!(sem_id > 0);
    debug!("[semop] sembuf: {:?}", sem_buf);

    let sem = {
        let sem_sets = sem_sets();
        let sem_set = sem_sets.get(&sem_id).ok_or(Error::new(Errno::EINVAL))?;
        // TODO: Support permission check
        warn!("Semaphore operation doesn't support permission check now");

        sem_set
            .get(sem_buf.sem_num as usize)
            .ok_or(Error::new(Errno::EFBIG))?
            .clone()
    };

    sem.sem_op(sem_buf, timeout, ctx)
}
