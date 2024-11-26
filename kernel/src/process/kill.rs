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
use crate::{prelude::*, thread::Tid};

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
            ctx.posix_thread.enqueue_signal(Box::new(signal));
            return Ok(());
        }

        return kill_process(ctx.process, Some(signal), ctx);
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
        .ok_or_else(|| Error::with_message(Errno::ESRCH, "target group does not exist"))?;

    let inner = process_group.inner.lock();
    for process in inner.processes.values() {
        kill_process(process, signal, ctx)?;
    }

    Ok(())
}

/// Sends a signal to a target thread, using the current process
/// as the sender.
///
/// If `signal` is `None`, this method will only check permission without sending
/// any signal.
pub fn tgkill(tid: Tid, tgid: Pid, signal: Option<UserSignal>, ctx: &Context) -> Result<()> {
    let thread = thread_table::get_thread(tid)
        .ok_or_else(|| Error::with_message(Errno::ESRCH, "target thread does not exist"))?;

    if thread.is_exited() {
        return Ok(());
    }

    let posix_thread = thread.as_posix_thread().unwrap();

    // Check tgid
    let pid = posix_thread.process().pid();
    if pid != tgid {
        return_errno_with_message!(
            Errno::EINVAL,
            "the combination of tgid and pid is not valid"
        );
    }

    // Check permission
    let signum = signal.map(|signal| signal.num());
    let sender = current_thread_sender_ids(signum.as_ref(), ctx);
    posix_thread.check_signal_perm(signum.as_ref(), &sender)?;

    if let Some(signal) = signal {
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
    let current = current!();
    for process in process_table::process_table_mut().iter() {
        if Arc::ptr_eq(&current, process) || process.is_init_process() {
            continue;
        }

        kill_process(process, signal, ctx)?;
    }

    Ok(())
}

fn kill_process(process: &Process, signal: Option<UserSignal>, ctx: &Context) -> Result<()> {
    let tasks = process.tasks().lock();

    let signum = signal.map(|signal| signal.num());
    let sender_ids = current_thread_sender_ids(signum.as_ref(), ctx);

    let mut permitted_thread = None;
    for task in tasks.iter() {
        let posix_thread = task.as_posix_thread().unwrap();

        // First check permission
        if posix_thread
            .check_signal_perm(signum.as_ref(), &sender_ids)
            .is_ok()
        {
            let Some(ref signum) = signum else {
                // If signal is None, only permission check is required
                return Ok(());
            };

            if !posix_thread.has_signal_blocked(*signum) {
                // Send signal to any thread that does not blocks the signal.
                let signal = signal.unwrap();
                posix_thread.enqueue_signal(Box::new(signal));
                return Ok(());
            } else if permitted_thread.is_none() {
                permitted_thread = Some(posix_thread);
            }
        }
    }

    let Some(permitted_thread) = permitted_thread else {
        return_errno_with_message!(Errno::EPERM, "cannot send signal to the target process");
    };

    // If signal is None, only permission check is required
    let Some(signal) = signal else { return Ok(()) };

    // If all threads block the signal, send signal to the first thread.
    permitted_thread.enqueue_signal(Box::new(signal));

    Ok(())
}

fn current_thread_sender_ids(signum: Option<&SigNum>, ctx: &Context) -> SignalSenderIds {
    let credentials = ctx.posix_thread.credentials();
    let ruid = credentials.ruid();
    let euid = credentials.euid();
    let sid = signum.and_then(|signum| {
        if *signum == SIGCONT {
            Some(ctx.process.session().unwrap().sid())
        } else {
            None
        }
    });

    SignalSenderIds::new(ruid, euid, sid)
}

/// The ids of the signal sender process.
///
/// This struct now includes effective user id, real user id and session id.
pub(super) struct SignalSenderIds {
    ruid: Uid,
    euid: Uid,
    sid: Option<Sid>,
}

impl SignalSenderIds {
    fn new(ruid: Uid, euid: Uid, sid: Option<Sid>) -> Self {
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
