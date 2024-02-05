// SPDX-License-Identifier: MPL-2.0

use super::posix_thread::PosixThreadExt;
use super::signal::signals::user::UserSignal;
use super::signal::signals::Signal;
use super::{credentials, process_table, Pgid, Pid, Process, Sid, Uid};
use crate::prelude::*;
use crate::thread::{thread_table, Tid};

/// Sends a signal to a process, using the current process as the sender.
///
/// The credentials of the current process will be checked to determine
/// if it is authorized to send the signal to this particular target process.
///
/// If `signal` is `None`, this method will only check permission without sending
/// any signal.
pub fn kill(pid: Pid, signal: Option<UserSignal>) -> Result<()> {
    let process = process_table::get_process(&pid)
        .ok_or_else(|| Error::with_message(Errno::ESRCH, "the target process does not exist"))?;

    kill_process(&process, signal)
}

/// Sends a signal to all processes in a group, using the current process
/// as the sender.
///
/// The credentials of the current process will be checked to determine
/// if it is authorized to send the signal to the target group.
///
/// If `signal` is `None`, this method will only check permission without sending
/// any signal.
pub fn kill_group(pgid: Pgid, signal: Option<UserSignal>) -> Result<()> {
    let process_group = process_table::get_process_group(&pgid)
        .ok_or_else(|| Error::with_message(Errno::ESRCH, "target group does not exist"))?;

    let inner = process_group.inner.lock();
    for process in inner.processes.values() {
        kill_process(process, signal)?;
    }

    Ok(())
}

/// Sends a signal to a target thread, using the current process
/// as the sender.
///
/// If `signal` is `None`, this method will only check permission without sending
/// any signal.
pub fn tgkill(tid: Tid, tgid: Pid, signal: Option<UserSignal>) -> Result<()> {
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
    let sender = current_thread_sender_ids();
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
pub fn kill_all(signal: Option<UserSignal>) -> Result<()> {
    let current = current!();
    let processes = process_table::get_all_processes();
    for process in processes {
        if Arc::ptr_eq(&current, &process) || process.is_init_process() {
            continue;
        }

        kill_process(&process, signal)?;
    }

    Ok(())
}

fn kill_process(process: &Process, signal: Option<UserSignal>) -> Result<()> {
    let threads = process.threads().lock();
    let posix_threads = threads
        .iter()
        .map(|thread| thread.as_posix_thread().unwrap());

    // First check permission
    let signum = signal.map(|signal| signal.num());
    let sender_ids = current_thread_sender_ids();
    let mut permitted_threads = {
        posix_threads.clone().filter(|posix_thread| {
            posix_thread
                .check_signal_perm(signum.as_ref(), &sender_ids)
                .is_ok()
        })
    };

    if permitted_threads.clone().count() == 0 {
        return_errno_with_message!(Errno::EPERM, "cannot send signal to the target process");
    }

    let Some(signal) = signal else { return Ok(()) };

    // Send signal to any thread that does not blocks the signal.
    for thread in permitted_threads.clone() {
        if !thread.has_signal_blocked(&signal) {
            thread.enqueue_signal(Box::new(signal));
            return Ok(());
        }
    }

    // If all threads block the signal, send signal to the first thread.
    let first_thread = permitted_threads.next().unwrap();
    first_thread.enqueue_signal(Box::new(signal));

    Ok(())
}

fn current_thread_sender_ids() -> SignalSenderIds {
    let credentials = credentials();
    let ruid = credentials.ruid();
    let euid = credentials.euid();
    let sid = current!().session().unwrap().sid();
    SignalSenderIds::new(ruid, euid, sid)
}

/// The ids of the signal sender process.
///
/// This struct now includes effective user id, real user id and session id.
pub(super) struct SignalSenderIds {
    ruid: Uid,
    euid: Uid,
    sid: Sid,
}

impl SignalSenderIds {
    fn new(ruid: Uid, euid: Uid, sid: Sid) -> Self {
        Self { ruid, euid, sid }
    }

    pub(super) fn ruid(&self) -> Uid {
        self.ruid
    }

    pub(super) fn euid(&self) -> Uid {
        self.euid
    }

    pub(super) fn sid(&self) -> Sid {
        self.sid
    }
}
