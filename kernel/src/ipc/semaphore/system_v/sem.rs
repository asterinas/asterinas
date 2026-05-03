// SPDX-License-Identifier: MPL-2.0

use core::{
    slice::Iter,
    sync::atomic::{AtomicU16, Ordering},
    time::Duration,
};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;
use ostd::sync::{Waiter, Waker};

use super::{
    PermissionMode,
    sem_set::{SEMVMX, SemSetInner},
};
use crate::{
    ipc::{IpcFlags, IpcId, IpcNamespace},
    prelude::*,
    process::Pid,
};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct SemBuf {
    sem_num: u16,
    sem_op: i16,
    sem_flags: i16,
}

impl SemBuf {
    pub(super) fn sem_num(&self) -> u16 {
        self.sem_num
    }
}

#[repr(u16)]
#[derive(Clone, Copy, Debug, TryFromInt)]
pub(super) enum Status {
    Normal = 0,
    Pending = 1,
    Removed = 2,
}

impl From<Status> for u16 {
    fn from(value: Status) -> Self {
        value as u16
    }
}

define_atomic_version_of_integer_like_type!(Status, try_from = true, {
    struct AtomicStatus(AtomicU16);
});

/// A pending semaphore operation.
pub(super) struct PendingOp {
    sops: Vec<SemBuf>,
    status: Arc<AtomicStatus>,
    waker: Option<Arc<Waker>>,
    pid: Pid,
}

impl PendingOp {
    pub(super) fn sops_iter(&self) -> Iter<'_, SemBuf> {
        self.sops.iter()
    }

    pub(super) fn set_status(&self, status: Status) {
        self.status.store(status, Ordering::Relaxed);
    }

    pub(super) fn waker(&self) -> Option<&Arc<Waker>> {
        self.waker.as_ref()
    }
}

impl Debug for PendingOp {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PendingOp")
            .field("sops", &self.sops)
            .field("status", &(self.status.load(Ordering::Relaxed)))
            .field("pid", &self.pid)
            .finish()
    }
}

#[derive(Debug)]
pub struct Semaphore {
    val: i32,
    /// The PID of the process that last modified the semaphore.
    ///
    /// This includes the following cases:
    /// - through `semop` with a non-zero `sem_op`,
    /// - through `semctl` with `SETVAL` and `SETALL`, and
    /// - through `SEM_UNDO` on process exit.
    latest_modified_pid: Pid,
}

impl Semaphore {
    pub(super) fn set_val(&mut self, val: i32) {
        self.val = val;
    }

    pub fn val(&self) -> i32 {
        self.val
    }

    pub(super) fn set_latest_modified_pid(&mut self, pid: Pid) {
        self.latest_modified_pid = pid;
    }

    pub fn latest_modified_pid(&self) -> Pid {
        self.latest_modified_pid
    }

    pub(super) fn new(val: i32) -> Self {
        Self {
            val,
            latest_modified_pid: current!().pid(),
        }
    }
}

pub fn sem_op(
    sem_id: IpcId,
    sops: Vec<SemBuf>,
    timeout: Option<Duration>,
    ipc_ns: &Arc<IpcNamespace>,
    ctx: &Context,
) -> Result<()> {
    let has_dup = check_dup_sops(&sops);
    if has_dup {
        warn!("Multiple operations on the same semaphore are not supported");
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "multiple operations on the same semaphore are not supported"
        );
    }

    let is_alter = check_alter_sop(&sops);

    let mut pending_op = PendingOp {
        sops,
        status: Arc::new(AtomicStatus::new(Status::Pending)),
        waker: None,
        pid: ctx.process.pid(),
    };

    // TODO: Support permission check
    warn!("Semaphore operation doesn't support permission check now");

    enum SemOpResult {
        Completed,
        Pending {
            status: Arc<AtomicStatus>,
            waiter: Waiter,
        },
    }

    let sem_op_result = ipc_ns.with_sem_set(sem_id, PermissionMode::empty(), |sem_set| {
        let mut inner = sem_set.inner();

        if perform_atomic_semop(&mut inner.sems, &mut pending_op)? {
            if is_alter {
                let wake_queue = do_smart_update(&mut inner, &pending_op);
                for wake_op in wake_queue {
                    wake_op.set_status(Status::Normal);
                    if let Some(waker) = wake_op.waker {
                        waker.wake_up();
                    }
                }
            }

            sem_set.update_otime();
            return Ok(SemOpResult::Completed);
        }

        // Prepare to wait
        let status = pending_op.status.clone();
        let (waiter, waker) = Waiter::new_pair();
        pending_op.waker = Some(waker);

        // Insert the operation to the pending list
        if is_alter {
            inner.pending_alter.push_back(pending_op);
        } else {
            inner.pending_const.push_back(pending_op);
        }

        Ok(SemOpResult::Pending { status, waiter })
    })?;

    let (result, status) = match sem_op_result {
        SemOpResult::Completed => return Ok(()),
        SemOpResult::Pending { status, waiter } => {
            let result = waiter.pause_timeout(&timeout.as_ref().into());
            (result, status)
        }
    };

    if matches!(status.load(Ordering::Relaxed), Status::Pending) {
        // Remove and check again to avoid race conditions
        let _ = ipc_ns.with_sem_set(sem_id, PermissionMode::empty(), |sem_set| {
            let mut inner = sem_set.inner();
            let pending_ops = if is_alter {
                &mut inner.pending_alter
            } else {
                &mut inner.pending_const
            };
            // FIXME: This may be time-consuming
            pending_ops.retain(|op| !Arc::ptr_eq(&op.status, &status));

            Ok(())
        });
    }

    match status.load(Ordering::Relaxed) {
        Status::Normal => Ok(()),
        Status::Removed => {
            return_errno_with_message!(Errno::EIDRM, "the semaphore set is removed");
        }
        Status::Pending => {
            if let Err(err) = result
                && err.error() == Errno::ETIME
            {
                return_errno_with_message!(Errno::EAGAIN, "the time limit is reached");
            } else {
                return_errno_with_message!(
                    Errno::EINTR,
                    "the current thread is interrupted by a signal"
                )
            }
        }
    }
}

/// Checks whether there are two operations that will operate on the same semaphore and the first
/// operation is an alteration operation.
fn check_dup_sops(sops: &[SemBuf]) -> bool {
    fn check_dup_slow(sops: &[SemBuf]) -> bool {
        for (i, op_i) in sops.iter().enumerate() {
            if op_i.sem_op == 0 {
                continue;
            }
            for op_j in sops[i + 1..].iter() {
                if op_i.sem_num == op_j.sem_num {
                    return true;
                }
            }
        }
        false
    }

    let mut mask = 0;
    for op in sops.iter() {
        let bit = 1u64 << (op.sem_num % 64);
        if mask & bit != 0 {
            return check_dup_slow(sops);
        }
        if op.sem_op != 0 {
            mask |= bit;
        }
    }
    false
}

/// Checks whether there is an operation that will change the value of a semaphore.
fn check_alter_sop(sops: &[SemBuf]) -> bool {
    sops.iter().any(|op| op.sem_op != 0)
}

/// Looks for operations that can be completed after an alteration operation and then completes
/// them.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.0.9/source/ipc/sem.c#L1029>
fn do_smart_update(inner: &mut SemSetInner, pending_op: &PendingOp) -> LinkedList<PendingOp> {
    let mut wake_queue = LinkedList::new();

    let (sems, pending_alter, pending_const) = inner.field_mut();

    if !pending_const.is_empty() {
        do_smart_wakeup_zero(sems, pending_const, pending_op, &mut wake_queue);
    }
    if !pending_alter.is_empty() {
        update_pending_alter(sems, pending_alter, pending_const, &mut wake_queue);
    }

    wake_queue
}

/// Looks for alteration operations that can be completed and then completes them.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.0.9/source/ipc/sem.c#L949>
pub(super) fn update_pending_alter(
    sems: &mut [Semaphore],
    pending_alter: &mut LinkedList<PendingOp>,
    pending_const: &mut LinkedList<PendingOp>,
    wake_queue: &mut LinkedList<PendingOp>,
) {
    let mut cursor = pending_alter.cursor_front_mut();
    while let Some(alter_op) = cursor.current() {
        if let Ok(true) = perform_atomic_semop(sems, alter_op) {
            let mut alter_op = cursor.remove_current_as_list().unwrap();

            do_smart_wakeup_zero(sems, pending_const, alter_op.front().unwrap(), wake_queue);

            wake_queue.append(&mut alter_op);
        } else {
            cursor.move_next();
        }
    }
}

/// Wakes up pending tasks on constant operations if an alteration operation made those constant
/// operations possible to complete.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.0.9/source/ipc/sem.c#L893>
fn do_smart_wakeup_zero(
    sems: &mut [Semaphore],
    pending_const: &mut LinkedList<PendingOp>,
    pending_op: &PendingOp,
    wake_queue: &mut LinkedList<PendingOp>,
) {
    for sop in pending_op.sops_iter() {
        if sems.get(sop.sem_num as usize).unwrap().val == 0 {
            wake_const_ops(sems, pending_const, wake_queue);
            return;
        }
    }
}

/// Wakes up pending tasks on constant operations if they can be completed.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.0.9/source/ipc/sem.c#L854>
pub(super) fn wake_const_ops(
    sems: &mut [Semaphore],
    pending_const: &mut LinkedList<PendingOp>,
    wake_queue: &mut LinkedList<PendingOp>,
) {
    let mut cursor = pending_const.cursor_front_mut();
    while let Some(const_op) = cursor.current() {
        if let Ok(true) = perform_atomic_semop(sems, const_op) {
            wake_queue.append(&mut cursor.remove_current_as_list().unwrap());
        } else {
            cursor.move_next();
        }
    }
}

/// Performs atomic semaphore operations.
///
/// 1. Return `Ok(true)` if all the operations succeed.
/// 2. Return `Ok(false)` if the caller needs to wait.
/// 3. Return `Err(err)` if the operations cause an error.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.0.9/source/ipc/sem.c#L719>
fn perform_atomic_semop(sems: &mut [Semaphore], pending_op: &mut PendingOp) -> Result<bool> {
    for op in pending_op.sops_iter() {
        let flags = IpcFlags::from_bits_truncate(op.sem_flags as u32);

        let Some(sem) = sems.get(op.sem_num as usize) else {
            return_errno_with_message!(Errno::EFBIG, "the semaphore number is out of bounds");
        };
        let mut result = sem.val();

        // Zero condition
        if op.sem_op == 0 && result != 0 {
            if flags.contains(IpcFlags::IPC_NOWAIT) {
                return_errno_with_message!(Errno::EAGAIN, "the semaphore value is not zero");
            } else {
                return Ok(false);
            }
        }

        result += i32::from(op.sem_op);
        if result < 0 {
            if flags.contains(IpcFlags::IPC_NOWAIT) {
                return_errno_with_message!(Errno::EAGAIN, "the semaphore value is too small");
            } else {
                return Ok(false);
            }
        }

        if result > SEMVMX {
            return_errno_with_message!(Errno::ERANGE, "semaphore value exceeds SEMVMX");
        }
        if flags.contains(IpcFlags::SEM_UNDO) {
            todo!()
        }
    }

    // Success, do operation
    for op in pending_op.sops_iter() {
        let sem = &mut sems[op.sem_num as usize];
        if op.sem_op != 0 {
            sem.val += i32::from(op.sem_op);
            sem.latest_modified_pid = pending_op.pid;
        }
    }

    Ok(true)
}
