// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::{
    abi::{SockFilter, SockFprog},
    cbpf::{MAX_INSNS, VerifiedProgram},
    filter::FilterLogging,
    listener::{Listener, ListenerFile, NotificationWaitMode},
};
use crate::{
    fs::file::file_table::{FdFlags, FileDescReservation},
    prelude::*,
    process::{ResourceType, credentials::capabilities::CapSet, posix_thread::AsPosixThread},
    security::lsm::hooks::{self, CapableContext},
    thread::AsThread,
};

const FILTER_FLAG_TSYNC: u32 = 1;
const FILTER_FLAG_LOG: u32 = 1 << 1;
// Opts out of additional speculation mitigation on filter installation.
// No supported architecture currently enables such a mitigation in seccomp.
const FILTER_FLAG_SPEC_ALLOW: u32 = 1 << 2;
const FILTER_FLAG_NEW_LISTENER: u32 = 1 << 3;
const FILTER_FLAG_TSYNC_ESRCH: u32 = 1 << 4;
const FILTER_FLAG_WAIT_KILLABLE_RECV: u32 = 1 << 5;

/// Enters strict mode, serialized against filter installation and TSYNC.
pub(super) fn install_strict(ctx: &Context) -> Result<()> {
    let _tasks = ctx.process.tasks().lock();
    let state = ctx.posix_thread.seccomp_snapshot().to_strict()?;
    ctx.posix_thread.set_seccomp(state);
    Ok(())
}

/// Validates a new filter and any requested thread synchronization before publishing the policy.
///
/// Returns `Ok(0)` on successful installation, or the listener file descriptor if
/// `NEW_LISTENER` was requested. If `TSYNC` finds an incompatible thread,
/// rejects the installation and returns `Ok(tid)` for the first such thread,
/// or reports `ESRCH` if `TSYNC_ESRCH` was requested.
pub(super) fn install_filter(ctx: &Context, flags: u32, fprog_addr: Vaddr) -> Result<isize> {
    if flags
        & !(FILTER_FLAG_TSYNC
            | FILTER_FLAG_LOG
            | FILTER_FLAG_SPEC_ALLOW
            | FILTER_FLAG_NEW_LISTENER
            | FILTER_FLAG_TSYNC_ESRCH
            | FILTER_FLAG_WAIT_KILLABLE_RECV)
        != 0
    {
        return_errno_with_message!(Errno::EINVAL, "unsupported seccomp filter flags");
    }
    let sync_threads = flags & FILTER_FLAG_TSYNC != 0;
    let new_listener = flags & FILTER_FLAG_NEW_LISTENER != 0;
    if sync_threads && new_listener && flags & FILTER_FLAG_TSYNC_ESRCH == 0 {
        return_errno_with_message!(
            Errno::EINVAL,
            "a synchronized listener requires TSYNC_ESRCH"
        );
    }

    let wait_mode = if flags & FILTER_FLAG_WAIT_KILLABLE_RECV != 0 {
        if !new_listener {
            return_errno_with_message!(
                Errno::EINVAL,
                "killable notification waiting requires a new listener"
            );
        }
        NotificationWaitMode::KillableAfterReceive
    } else {
        NotificationWaitMode::Interruptible
    };

    let user_space = ctx.user_space();
    let fprog: SockFprog = user_space.read_val(fprog_addr)?;
    let len = fprog.len as usize;
    if len == 0 || len > MAX_INSNS {
        return_errno_with_message!(Errno::EINVAL, "invalid seccomp program length");
    }
    check_install_permission(ctx)?;
    if fprog.filter == 0 {
        return_errno_with_message!(Errno::EINVAL, "null seccomp instruction array");
    }

    let mut raw = Vec::<SockFilter>::new();
    raw.try_reserve_exact(len)
        .map_err(|_| Error::with_message(Errno::ENOMEM, "cannot allocate seccomp program"))?;
    raw.resize(
        len,
        SockFilter {
            code: 0,
            jt: 0,
            jf: 0,
            k: 0,
        },
    );
    user_space.read_bytes(fprog.filter as Vaddr, raw.as_mut_bytes())?;
    let program = VerifiedProgram::new(raw)?;

    // Reserve before policy validation, but do not expose the file until all
    // threads have received the new policy. Failure releases the reservation.
    let listener = if new_listener {
        let limit = ctx
            .process
            .resource_limits()
            .get_rlimit(ResourceType::RLIMIT_NOFILE)
            .get_cur();
        let reservation = {
            let table = ctx.thread_local.borrow_file_table();
            FileDescReservation::new(&table, limit)?
        };
        let state = Listener::new(wait_mode)?;
        let file = ListenerFile::new(state.clone())?;
        Some((reservation, file, state))
    } else {
        None
    };

    // Linux accepts TSYNC_ESRCH without TSYNC; it has no effect in that case.
    // Lock order: credential guard -> task set -> per-thread seccomp state.
    // Ordinary installs share the task-set lock, so an append cannot lose a
    // concurrent TSYNC update. User copies and verification precede all locks.
    let _cred_guard = if sync_threads {
        Some(ctx.process.cred_guard().lock_for_seccomp()?)
    } else {
        None
    };
    let tasks = ctx.process.tasks().lock();
    let old_state = ctx.posix_thread.seccomp_snapshot();
    let logging = if flags & FILTER_FLAG_LOG != 0 {
        FilterLogging::Requested
    } else {
        FilterLogging::Default
    };
    let new_state = old_state.append_with_listener(
        program,
        logging,
        listener.as_ref().map(|(_, _, state)| state.clone()),
    )?;

    if sync_threads {
        for task in tasks.as_slice() {
            if task.as_thread().unwrap().is_exited() {
                continue;
            }
            let target = task.as_posix_thread().unwrap();
            if !target.seccomp_snapshot().is_ancestor_of(&old_state) {
                if flags & FILTER_FLAG_TSYNC_ESRCH != 0 {
                    return_errno_with_message!(
                        Errno::ESRCH,
                        "seccomp thread synchronization failed"
                    );
                }
                return Ok(target.tid() as isize);
            }
        }
    }

    // No fallible operations remain after the first policy update.
    ctx.posix_thread.set_seccomp(new_state);
    if sync_threads && !tasks.has_exited_group() {
        for task in tasks.as_slice() {
            if task.as_thread().unwrap().is_exited() {
                continue;
            }
            let target = task.as_posix_thread().unwrap();
            if !core::ptr::eq(target, ctx.posix_thread) {
                target.inherit_seccomp(ctx.posix_thread);
            }
        }
    }
    drop(tasks);
    drop(_cred_guard);
    if let Some((reservation, file, _)) = listener {
        Ok(reservation.install(file, FdFlags::CLOEXEC).into())
    } else {
        Ok(0)
    }
}

fn check_install_permission(ctx: &Context) -> Result<()> {
    if ctx.posix_thread.credentials().no_new_privs() {
        return Ok(());
    }

    let user_ns = ctx.thread_local.borrow_user_ns();
    hooks::on_capable(CapableContext::new(
        &user_ns,
        ctx.posix_thread,
        CapSet::SYS_ADMIN,
    ))
    .map_err(|_| Error::with_message(Errno::EACCES, "seccomp installation is not permitted"))
}
