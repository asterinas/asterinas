// SPDX-License-Identifier: MPL-2.0

use alloc::vec::Vec;
use core::time::Duration;

use ostd::sync::{Mutex, Waiter, Waker};

use super::{key_t, sem_set::SEMVMX};
use crate::{
    ipc::{sem_set::sem_sets, IpcFlags},
    prelude::*,
    process::{Pid, Process},
    time::{clocks::JIFFIES_TIMER_MANAGER, timer::Timeout, timespec_t},
};

#[derive(Clone, Copy, Debug, Pod)]
#[repr(C)]
pub struct SemBuf {
    pub sem_num: u16,
    pub sem_op: i16,
    pub sem_flags: i16,
}

#[repr(u16)]
#[derive(Debug)]
enum Status {
    Normal = 0,
    Pending = 1,
    Removed = 2,
}

struct PendingOp {
    sem_buf: SemBuf,
    status: Arc<Mutex<Status>>,
    waker: Arc<Waker>,
    pid: Pid,
    process: Weak<Process>,
}

impl Debug for PendingOp {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PendingOp")
            .field("sem_buf", &self.sem_buf)
            .finish()
    }
}

#[derive(Debug)]
pub struct Semaphore {
    val: Mutex<i32>,
    /// Pending operations. For each pending operation, it has `sem_op <= 0`.
    pending_ops: Mutex<Vec<PendingOp>>,
}

impl Semaphore {
    pub fn set_val(&self, val: i32) -> Result<()> {
        if !(0..SEMVMX).contains(&val) {
            return_errno!(Errno::ERANGE);
        }

        let mut current_val = self.val.lock();
        *current_val = val;

        self.update_pending_ops(current_val);
        Ok(())
    }

    pub fn val(&self) -> i32 {
        *self.val.lock()
    }

    pub(super) fn new(val: i32) -> Self {
        Self {
            val: Mutex::new(val),
            pending_ops: Mutex::new(Vec::new()),
        }
    }

    fn sem_op(&self, sem_buf: SemBuf, timeout: Option<timespec_t>) -> Result<()> {
        let mut val = self.val.lock();
        let sem_op = sem_buf.sem_op;

        let flags = IpcFlags::from_bits(sem_buf.sem_flags as u32).unwrap();
        if flags.contains(IpcFlags::SEM_UNDO) {
            todo!()
        }

        // Operate val
        let positive_condition = sem_op.is_positive();
        let negative_condition = sem_op.is_negative() && sem_op.abs() as i32 <= *val;
        let zero_condition = sem_op == 0 && *val == 0;

        if positive_condition {
            if *val + i32::from(sem_op) > SEMVMX {
                return_errno!(Errno::ERANGE);
            }
            *val += i32::from(sem_op);
            self.update_pending_ops(val);
            return Ok(());
        } else if negative_condition {
            *val += i32::from(sem_op);
            return Ok(());
        } else if zero_condition {
            return Ok(());
        }

        // Need to wait for the semaphore
        if flags.contains(IpcFlags::IPC_NOWAIT) {
            return_errno!(Errno::EAGAIN);
        }

        // Add current to pending list
        let (waiter, waker) = Waiter::new_pair();
        let status = Arc::new(Mutex::new(Status::Pending));
        let current = current!();
        let pid = current.pid();
        let pending_op = PendingOp {
            sem_buf,
            status: status.clone(),
            waker: waker.clone(),
            process: Arc::downgrade(&current),
            pid,
        };
        self.pending_ops.lock().push(pending_op);
        drop(current);
        drop(val);

        // Wait
        if let Some(timeout) = timeout {
            let jiffies_timer = JIFFIES_TIMER_MANAGER.get().unwrap().create_timer(move || {
                waker.wake_up();
            });
            jiffies_timer.set_timeout(Timeout::After(Duration::from(timeout)));
        }
        waiter.wait();

        // Check status and return
        let status_guard = status.lock();
        match *status_guard {
            Status::Normal => Ok(()),
            Status::Removed => Err(Error::new(Errno::EIDRM)),
            Status::Pending => {
                let mut pending_ops = self.pending_ops.lock();
                pending_ops.retain(|op| op.pid != pid);
                Err(Error::new(Errno::EAGAIN))
            }
        }
    }

    /// Update pending ops after the val changed.
    fn update_pending_ops(&self, mut val: MutexGuard<i32>) {
        debug_assert!(*val >= 0);
        trace!("Updating pending ops, semaphore before: {:?}", *val);

        // Two steps:
        // 1. Remove the pending_op with `sem_op < 0` if it can.
        // 2. If val is equal to 0, then remove the pending_op with `sem_op = 0`

        // Step one:
        let mut pending_ops = self.pending_ops.lock();
        pending_ops.retain_mut(|op| {
            // Check if the process alive.
            if op.process.upgrade().is_none() {
                return false;
            }
            debug_assert!(op.sem_buf.sem_op <= 0);

            if op.sem_buf.sem_op.abs() as i32 <= *val {
                trace!(
                    "Found removable pending op, op: {:?}, pid:{:?}",
                    op.sem_buf.sem_op,
                    op.pid
                );

                *val += i32::from(op.sem_buf.sem_op);
                *op.status.lock() = Status::Normal;
                op.waker.wake_up();
                false
            } else {
                true
            }
        });

        // Step two:
        if *val == 0 {
            pending_ops.retain_mut(|op| {
                // Check if the process alive.
                if op.process.upgrade().is_none() {
                    return false;
                }

                if op.sem_buf.sem_op == 0 {
                    trace!("Found removable pending op, op: 0, pid:{:?}", op.pid);

                    *op.status.lock() = Status::Normal;
                    op.waker.wake_up();
                    false
                } else {
                    true
                }
            });
        }
        trace!("Updated pending ops, semaphore after: {:?}", *val);
    }
}

impl Drop for Semaphore {
    fn drop(&mut self) {
        let mut pending_ops = self.pending_ops.lock();
        for pending_op in pending_ops.iter_mut() {
            *pending_op.status.lock() = Status::Removed;
            pending_op.waker.wake_up();
        }
    }
}

pub fn sem_op(sem_id: key_t, sem_buf: SemBuf, timeout: Option<timespec_t>) -> Result<()> {
    debug_assert!(sem_id > 0);
    debug!("[semop] sembuf: {:?}", sem_buf);

    let sem_sets = sem_sets();
    let sem_set = sem_sets.get(&sem_id).ok_or(Error::new(Errno::EINVAL))?;
    // TODO: Support permission check
    warn!("Semaphore operation doesn't support permission check now");
    sem_set.update_otime();

    let sem = sem_set
        .get(sem_buf.sem_num as usize)
        .ok_or(Error::new(Errno::EINVAL))?;

    sem.sem_op(sem_buf, timeout)
}
