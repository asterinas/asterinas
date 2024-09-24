// SPDX-License-Identifier: MPL-2.0

use core::{
    slice::Iter,
    sync::atomic::{AtomicU16, Ordering},
    time::Duration,
};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;
use ostd::sync::{PreemptDisabled, Waiter, Waker};

use super::sem_set::{SemSetInner, SEMVMX};
use crate::{
    ipc::{key_t, semaphore::system_v::sem_set::sem_sets, IpcFlags},
    prelude::*,
    process::Pid,
    time::{clocks::JIFFIES_TIMER_MANAGER, timer::Timeout},
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
pub enum Status {
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

/// Pending atomic semop.
pub struct PendingOp {
    sops: Vec<SemBuf>,
    status: Arc<AtomicStatus>,
    waker: Option<Arc<Waker>>,
    pid: Pid,
}

impl PendingOp {
    pub fn sops_iter(&self) -> Iter<SemBuf> {
        self.sops.iter()
    }

    pub fn set_status(&self, status: Status) {
        self.status.store(status, Ordering::Relaxed);
    }

    pub fn waker(&self) -> &Option<Arc<Waker>> {
        &self.waker
    }

    pub fn pid(&self) -> Pid {
        self.pid
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
    /// PID of the process that last modified semaphore.
    /// - through semop with op != 0
    /// - through semctl with SETVAL and SETALL
    /// - through SEM_UNDO when task exit
    latest_modified_pid: Pid,
}

impl Semaphore {
    pub fn set_val(&mut self, val: i32) {
        self.val = val;
    }

    pub fn val(&self) -> i32 {
        self.val
    }

    pub fn set_latest_modified_pid(&mut self, pid: Pid) {
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
    sem_id: key_t,
    sops: Vec<SemBuf>,
    timeout: Option<Duration>,
    ctx: &Context,
) -> Result<()> {
    debug_assert!(sem_id > 0);
    debug!("[semop] sops: {:?}", sops);

    let pid = ctx.process.pid();
    let mut pending_op = PendingOp {
        sops,
        status: Arc::new(AtomicStatus::new(Status::Pending)),
        waker: None,
        pid,
    };

    // TODO: Support permission check
    warn!("Semaphore operation doesn't support permission check now");

    let (alter, dupsop) = get_sops_flags(&pending_op);
    if dupsop {
        warn!("Found duplicate sop");
    }

    let local_sem_sets = sem_sets();
    let sem_set = local_sem_sets
        .get(&sem_id)
        .ok_or(Error::new(Errno::EINVAL))?;
    let mut inner = sem_set.inner();

    if perform_atomic_semop(&mut inner.sems, &mut pending_op)? {
        if alter {
            let wake_queue = do_smart_update(&mut inner, &pending_op);
            for wake_op in wake_queue {
                wake_op.set_status(Status::Normal);
                if let Some(waker) = wake_op.waker {
                    waker.wake_up();
                }
            }
        }

        sem_set.update_otime();
        return Ok(());
    }

    // Prepare to wait
    let status = pending_op.status.clone();
    let (waiter, waker) = Waiter::new_pair();

    // Check if timeout exists to avoid calling `Arc::clone()`
    if let Some(timeout) = timeout {
        pending_op.waker = Some(waker.clone());

        let jiffies_timer = JIFFIES_TIMER_MANAGER.get().unwrap().create_timer(move || {
            waker.wake_up();
        });
        jiffies_timer.set_timeout(Timeout::After(timeout));
    } else {
        pending_op.waker = Some(waker);
    }

    if alter {
        inner.pending_alter.push_back(pending_op);
    } else {
        inner.pending_const.push_back(pending_op);
    }

    drop(inner);
    drop(local_sem_sets);

    waiter.wait();
    match status.load(Ordering::Relaxed) {
        Status::Normal => Ok(()),
        Status::Removed => Err(Error::new(Errno::EIDRM)),
        Status::Pending => {
            // FIXME: Getting sem_sets maybe time-consuming.
            let sem_sets = sem_sets();
            let sem_set = sem_sets.get(&sem_id).ok_or(Error::new(Errno::EINVAL))?;
            let mut inner = sem_set.inner();

            let pending_ops = if alter {
                &mut inner.pending_alter
            } else {
                &mut inner.pending_const
            };
            pending_ops.retain(|op| op.pid != pid);

            Err(Error::new(Errno::EAGAIN))
        }
    }
}

/// Update pending const and alter operations, ref: <https://elixir.bootlin.com/linux/v6.0.9/source/ipc/sem.c#L1029>
pub(super) fn do_smart_update(
    inner: &mut SpinLockGuard<SemSetInner, PreemptDisabled>,
    pending_op: &PendingOp,
) -> LinkedList<PendingOp> {
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

/// Look for pending alter operations that can be completed, ref: <https://elixir.bootlin.com/linux/v6.0.9/source/ipc/sem.c#L949>
pub(super) fn update_pending_alter(
    sems: &mut Box<[Semaphore]>,
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

/// Wakeup all wait for zero tasks, ref: <https://elixir.bootlin.com/linux/v6.0.9/source/ipc/sem.c#L893>
fn do_smart_wakeup_zero(
    sems: &mut Box<[Semaphore]>,
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

/// Wakeup pending const operations, ref: <https://elixir.bootlin.com/linux/v6.0.9/source/ipc/sem.c#L854>
pub(super) fn wake_const_ops(
    sems: &mut Box<[Semaphore]>,
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

/// Iter the sops and return the flags (alter, dupsop)
fn get_sops_flags(pending_op: &PendingOp) -> (bool, bool) {
    let mut alter = false;
    let mut dupsop = false;
    let mut dup = 0;
    for sop in pending_op.sops_iter() {
        let mask: u64 = 1 << ((sop.sem_num) % 64);

        if (dup & mask) != 0 {
            dupsop = true;
        }

        if sop.sem_op != 0 {
            alter = true;
            dup |= mask;
        }
    }
    (alter, dupsop)
}

/// Perform atomic semop, ref: <https://elixir.bootlin.com/linux/v6.0.9/source/ipc/sem.c#L719>
/// 1. Return Ok(true) if the operation success.
/// 2. Return Ok(false) if the caller needs to wait.
/// 3. Return Err(err) if the operation cause error.
fn perform_atomic_semop(sems: &mut Box<[Semaphore]>, pending_op: &mut PendingOp) -> Result<bool> {
    let mut result;
    for op in pending_op.sops_iter() {
        let sem = sems.get(op.sem_num as usize).ok_or(Errno::EFBIG)?;
        let flags = IpcFlags::from_bits_truncate(op.sem_flags as u32);
        result = sem.val();

        // Zero condition
        if op.sem_op == 0 && result != 0 {
            if flags.contains(IpcFlags::IPC_NOWAIT) {
                return_errno!(Errno::EAGAIN);
            } else {
                return Ok(false);
            }
        }

        result += i32::from(op.sem_op);
        if result < 0 {
            if flags.contains(IpcFlags::IPC_NOWAIT) {
                return_errno!(Errno::EAGAIN);
            } else {
                return Ok(false);
            }
        }

        if result > SEMVMX {
            return_errno!(Errno::ERANGE);
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
