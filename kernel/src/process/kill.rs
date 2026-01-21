// SPDX-License-Identifier: MPL-2.0

use super::{
    Pgid, Pid, Process,
    posix_thread::{AsPosixThread, thread_table},
    process_table,
    signal::{constants::SIGCONT, sig_num::SigNum, signals::Signal},
};
use crate::{
    prelude::*,
    process::{credentials::capabilities::CapSet, posix_thread::PosixThread},
    thread::Tid,
};

/// Sends a signal to a process, using the current process as the sender.
///
/// The credentials of the current process will be checked to determine
/// if it is authorized to send the signal to this particular target process.
///
/// If `signal` is `None`, this method will only check permission without sending
/// any signal.
pub fn kill(pid: Pid, signal: Option<Box<dyn Signal>>, ctx: &Context) -> Result<()> {
    // Fast path: If the signal is sent to self, we can skip most checks.
    if pid == ctx.process.pid() {
        let Some(signal) = signal else {
            return Ok(());
        };

        if !ctx.posix_thread.has_signal_blocked(signal.num()) {
            // Killing the current thread does not raise any permission issues.
            ctx.posix_thread.enqueue_signal(signal);
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
pub fn kill_group<S: Signal + Clone>(pgid: Pgid, signal: Option<S>, ctx: &Context) -> Result<()> {
    let process_group = process_table::get_process_group(&pgid)
        .ok_or_else(|| Error::with_message(Errno::ESRCH, "the target group does not exist"))?;

    let mut result = Ok(());

    let inner = process_group.lock();
    for process in inner.iter() {
        let res = kill_process(
            process,
            signal.clone().map(|s| Box::new(s) as Box<dyn Signal>),
            ctx,
        );
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
pub fn tgkill(tid: Tid, tgid: Pid, signal: Option<Box<dyn Signal>>, ctx: &Context) -> Result<()> {
    let thread = thread_table::get_thread(tid)
        .ok_or_else(|| Error::with_message(Errno::ESRCH, "the target thread does not exist"))?;
    let target_posix_thread = thread.as_posix_thread().unwrap();

    // Check the TGID
    let pid = target_posix_thread.process().pid();
    if pid != tgid {
        return_errno_with_message!(
            Errno::ESRCH,
            "the combination of the TGID and the TID is not valid"
        );
    }

    // Check permission
    let signum = signal.as_ref().map(|signal| signal.num());
    check_signal_perm(target_posix_thread, ctx, signum)?;

    if thread.is_exited() {
        return Ok(());
    }

    if let Some(signal) = signal {
        // We've checked the permission issues above.
        // FIXME: We should take some lock while checking the permission to avoid race conditions.
        target_posix_thread.enqueue_signal(signal);
    }

    Ok(())
}

/// Sends a signal to all processes except current process and init process, using
/// the current process as the sender.
///
/// The credentials of the current process will be checked to determine
/// if it is authorized to send the signal to the target group.
pub fn kill_all<S: Signal + Clone>(signal: Option<S>, ctx: &Context) -> Result<()> {
    let mut result = Ok(());

    for process in process_table::process_table_mut().iter() {
        if Arc::ptr_eq(&ctx.process, process) || process.is_init_process() {
            continue;
        }

        let res = kill_process(
            process,
            signal.clone().map(|s| Box::new(s) as Box<dyn Signal>),
            ctx,
        );
        if res.is_err_and(|err| err.error() != Errno::EPERM) {
            result = res;
        }
    }

    result
}

fn kill_process(process: &Process, signal: Option<Box<dyn Signal>>, ctx: &Context) -> Result<()> {
    let signum = signal.as_ref().map(|signal| signal.num());
    let target_main_thread = process.main_thread();
    check_signal_perm(target_main_thread.as_posix_thread().unwrap(), ctx, signum)?;

    if let Some(signal) = signal {
        process.enqueue_signal(signal);
    }

    Ok(())
}

// Reference: <https://elixir.bootlin.com/linux/v6.17/source/kernel/signal.c#L799>.
fn check_signal_perm(target: &PosixThread, ctx: &Context, signum: Option<SigNum>) -> Result<()> {
    let target_process = target.process();

    if Arc::ptr_eq(&target_process, &ctx.process) {
        return Ok(());
    }

    let current_cred = ctx.posix_thread.credentials();
    let target_cred = target.credentials();
    if current_cred.euid() == target_cred.suid()
        || current_cred.euid() == target_cred.ruid()
        || current_cred.ruid() == target_cred.suid()
        || current_cred.ruid() == target_cred.ruid()
    {
        return Ok(());
    }

    if target_process
        .user_ns()
        .lock()
        .check_cap(CapSet::KILL, ctx.posix_thread)
        .is_ok()
    {
        return Ok(());
    }

    if let Some(signum) = signum
        && signum == SIGCONT
    {
        let target_sid = target_process.sid();
        let current_sid = ctx.process.sid();
        if target_sid == current_sid {
            return Ok(());
        }
    }

    return_errno_with_message!(
        Errno::EPERM,
        "sending signal to the target process or thread is not allowed"
    )
}
