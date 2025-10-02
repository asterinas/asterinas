// SPDX-License-Identifier: MPL-2.0

use super::{
    posix_thread::{thread_table, AsPosixThread},
    process_table,
    signal::{
        constants::SIGCONT,
        sig_num::SigNum,
        signals::{user::UserSignal, Signal},
    },
    Pgid, Pid, Process, Sid, Uid,
};
use crate::{
    prelude::*,
    thread::{AsThread, Tid},
};

/// Sends a signal to a process, using the current process as the sender.
///
/// The credentials of the current process will be checked to determine
/// if it is authorized to send the signal to this particular target process.
///
/// If `signal` is `None`, this method will only check permission without sending
/// any signal.
pub fn kill(pid: Pid, signal: Option<UserSignal>, ctx: &Context) -> Result<()> {
    // Fast path: If the signal is sent to self, we can skip most check.
    if pid == ctx.process.pid() {
        let Some(signal) = signal else {
            return Ok(());
        };

        if !ctx.posix_thread.has_signal_blocked(signal.num()) {
            // Killing the current thread does not raise any permission issues.
            ctx.posix_thread.enqueue_signal(Box::new(signal));
            return Ok(());
        }

        return kill_process(ctx.process.as_ref(), Some(signal), ctx);
    }

    // Slow path

    let process = process_table::get_process(pid)
        .ok_or_else(|| Error::with_message(Errno::ESRCH, "the target process does not exist"))?;

    kill_process(&process, signal, ctx)
}

/// Sends a signal to all processes in a group, using the current process
/// as the sender.
///
/// The credentials of the current process will be checked to determine
/// if it is authorized to send the signal to the target group.
///
/// If `signal` is `None`, this method will only check permission without sending
/// any signal.
pub fn kill_group(pgid: Pgid, signal: Option<UserSignal>, ctx: &Context) -> Result<()> {
    let process_group = process_table::get_process_group(&pgid)
        .ok_or_else(|| Error::with_message(Errno::ESRCH, "the target group does not exist"))?;

    let mut result = Ok(());

    let inner = process_group.lock();
    for process in inner.iter() {
        let res = kill_process(process, signal, ctx);
        if res.is_err_and(|err| err.error() != Errno::EPERM) {
            result = res;
        }
    }

    result
}

/// Sends a signal to a target thread, using the current process
/// as the sender.
///
/// If `signal` is `None`, this method will only check permission without sending
/// any signal.
pub fn tgkill(tid: Tid, tgid: Pid, signal: Option<UserSignal>, ctx: &Context) -> Result<()> {
    let thread = thread_table::get_thread(tid)
        .ok_or_else(|| Error::with_message(Errno::ESRCH, "the target thread does not exist"))?;
    if thread.is_exited() {
        return Ok(());
    }

    let posix_thread = thread.as_posix_thread().unwrap();

    // Check the TGID
    let pid = posix_thread.process().pid();
    if pid != tgid {
        return_errno_with_message!(
            Errno::ESRCH,
            "the combination of the TGID and the TID is not valid"
        );
    }

    // Check permission
    let signum = signal.map(|signal| signal.num());
    let sender_ids = SignalSenderIds::for_current_thread(ctx, signum);
    posix_thread.check_signal_perm(signum.as_ref(), &sender_ids)?;

    if let Some(signal) = signal {
        // We've checked the permission issues above.
        // FIXME: We should take some lock while checking the permission to avoid race conditions.
        posix_thread.enqueue_signal(Box::new(signal));
    }

    Ok(())
}

/// Sends a signal to all processes except current process and init process, using
/// the current process as the sender.
///
/// The credentials of the current process will be checked to determine
/// if it is authorized to send the signal to the target group.
pub fn kill_all(signal: Option<UserSignal>, ctx: &Context) -> Result<()> {
    let mut result = Ok(());

    for process in process_table::process_table_mut().iter() {
        if Arc::ptr_eq(&ctx.process, process) || process.is_init_process() {
            continue;
        }

        let res = kill_process(process, signal, ctx);
        if res.is_err_and(|err| err.error() != Errno::EPERM) {
            result = res;
        }
    }

    result
}

fn kill_process(process: &Process, signal: Option<UserSignal>, ctx: &Context) -> Result<()> {
    let sig_dispositions = process.sig_dispositions().lock();
    let tasks = process.tasks().lock();

    let signum = signal.map(|signal| signal.num());
    let sender_ids = SignalSenderIds::for_current_thread(ctx, signum);

    let mut found_permitted_thread = false;
    let mut thread_to_enqueue = None;
    for task in tasks.as_slice() {
        let thread = task.as_thread().unwrap();
        let posix_thread = thread.as_posix_thread().unwrap();

        // Check permission
        if posix_thread
            .check_signal_perm(signum.as_ref(), &sender_ids)
            .is_err()
        {
            continue;
        }

        let Some(ref signum) = signum else {
            // If `signal` is `None`, only permission check is required.
            return Ok(());
        };

        found_permitted_thread = true;

        // FIXME: If the thread is exiting concurrently, it may miss the signal queued on it.
        if thread.is_exited() {
            continue;
        }

        if !posix_thread.has_signal_blocked(*signum) {
            // Send the signal to any alive thread that does not block the signal.
            thread_to_enqueue = Some(posix_thread);
            break;
        } else if thread_to_enqueue.is_none() {
            // If all threads block the signal, send it to the first permitted and alive thread.
            // FIXME: If it exits later with the signals still blocked, it will miss the signal
            // queued on it.
            thread_to_enqueue = Some(posix_thread);
        }
    }

    if !found_permitted_thread {
        return_errno_with_message!(
            Errno::EPERM,
            "the signal cannot be sent to the target process"
        );
    }

    let Some(thread_to_enqueue) = thread_to_enqueue else {
        // All threads have exited. This is a zombie process.
        return Ok(());
    };

    // Since `thread_to_enqueue` has been set, `signal` cannot be `None`.
    let signal = signal.unwrap();

    // Drop the signal if it's ignored. See explanation at `enqueue_signal_locked`.
    let signum = signal.num();
    if sig_dispositions.get(signum).will_ignore(signum) {
        return Ok(());
    }

    thread_to_enqueue.enqueue_signal_locked(Box::new(signal), sig_dispositions);

    Ok(())
}

/// The IDs of the signal sender process.
///
/// For all signals, this structure includes the sender thread's effective user ID and real user
/// ID. For SIGCONT, this structure additionally includes the sender thread's session ID.
pub(super) struct SignalSenderIds {
    ruid: Uid,
    euid: Uid,
    sid: Option<Sid>,
}

impl SignalSenderIds {
    pub(self) fn for_current_thread(ctx: &Context, signum: Option<SigNum>) -> Self {
        let credentials = ctx.posix_thread.credentials();
        let ruid = credentials.ruid();
        let euid = credentials.euid();

        let sid = signum.and_then(|signum| {
            if signum == SIGCONT {
                Some(ctx.process.sid())
            } else {
                None
            }
        });

        Self { ruid, euid, sid }
    }

    pub(super) fn ruid(&self) -> Uid {
        self.ruid
    }

    pub(super) fn euid(&self) -> Uid {
        self.euid
    }

    pub(super) fn sid(&self) -> Option<Sid> {
        self.sid
    }
}
