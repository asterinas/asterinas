// SPDX-License-Identifier: MPL-2.0

use ostd::{arch::cpu::context::UserContext, mm::VmIo, task::Task};

use super::{
    AsPosixThread, ThreadLocal,
    futex::{FutexVisibility, futex_wake},
    ptrace::PtraceEvent,
    robust_list::wake_robust_futex,
};
use crate::{
    context::current_userspace,
    prelude::*,
    process::{
        TermStatus,
        exit::{drop_after, exit_process},
        pid_table,
        signal::{
            constants::{SIGKILL, SIGSYS},
            sig_num::SigNum,
            signals::kernel::KernelSignal,
        },
        task_set::TaskSet,
    },
    thread::{AsThread, Tid},
};

/// Exits the current POSIX thread.
///
/// # Panics
///
/// If the current thread is not a POSIX thread, this method will panic.
pub(crate) fn do_exit(term_status: TermStatus, ctx: &Context, user_ctx: &mut UserContext) {
    exit_internal(term_status, ExitScope::Thread, ctx, user_ctx);
}

/// Kills all threads and exits the current POSIX process.
///
/// # Panics
///
/// If the current thread is not a POSIX thread, this method will panic.
pub(crate) fn do_exit_group(term_status: TermStatus, ctx: &Context, user_ctx: &mut UserContext) {
    exit_internal(term_status, ExitScope::Group, ctx, user_ctx);
}

/// Terminates the process with `SIGSYS` using the normal group-exit arbitration.
pub(crate) fn do_seccomp_exit_group(ctx: &Context, user_ctx: &mut UserContext) {
    do_seccomp_exit(SIGSYS, ExitScope::Group, ctx, user_ctx);
}

/// Terminates the current thread after a fatal seccomp decision.
pub(crate) fn do_seccomp_exit_thread(signal: SigNum, ctx: &Context, user_ctx: &mut UserContext) {
    do_seccomp_exit(signal, ExitScope::Thread, ctx, user_ctx);
}

fn do_seccomp_exit(signal: SigNum, scope: ExitScope, ctx: &Context, user_ctx: &mut UserContext) {
    {
        // Publish the dead mode before the ptrace exit stop and serialize it with TSYNC.
        let _tasks = ctx.process.tasks().lock();
        ctx.posix_thread.seccomp.lock().mark_dead();
    }
    exit_internal(TermStatus::Killed(signal), scope, ctx, user_ctx);
}

enum ExitScope {
    Thread,
    Group,
}

/// Exits the current POSIX thread or process.
//
// A mutable reference to `user_ctx` is needed because exiting a traced thread
// may trigger a "exit" ptrace-event-stop, which snapshots the current user
// register state and may later restore tracer-updated registers.
fn exit_internal(
    term_status: TermStatus,
    scope: ExitScope,
    ctx: &Context,
    user_ctx: &mut UserContext,
) {
    let current_task = Task::current().unwrap();
    let current_thread = current_task.as_thread().unwrap();
    let posix_thread = current_thread.as_posix_thread().unwrap();
    let thread_local = current_task.as_thread_local().unwrap();
    let posix_process = posix_thread.process();

    // Linux commits do_group_exit() before reporting PTRACE_EVENT_EXIT.
    // Otherwise a tracer could leave siblings running while the fatal thread
    // is stopped. Exec and group exit must arbitrate under the same lock.
    let exit_code = {
        let mut tasks = posix_process.tasks().lock();
        match scope {
            ExitScope::Thread => term_status.as_u32(),
            ExitScope::Group if tasks.has_exited_group() => posix_process.status().exit_code(),
            ExitScope::Group if tasks.in_execve() => 0,
            ExitScope::Group => {
                let exit_code = term_status.as_u32();
                posix_process.status().set_exit_code(exit_code);
                tasks.set_exited_group();
                sigkill_other_threads(&current_task, &tasks);
                exit_code
            }
        }
    };

    ctx.posix_thread
        .ptrace_may_stop_on(PtraceEvent::Exit(exit_code), ctx, user_ctx);

    let is_last_thread = {
        let mut tasks = posix_process.tasks().lock();

        // According to Linux's behavior, the last thread's exit code will become the process's
        // exit code, so here we should just overwrite the old value (if any).
        if !tasks.has_exited_group() && !tasks.in_execve() {
            posix_process.status().set_exit_code(exit_code);
        }

        posix_thread.set_exit_code(exit_code);

        // We should only change the thread status when running as the thread, so no race
        // conditions can occur in between.
        if current_thread.is_exited() {
            return;
        }
        current_thread.exit();

        tasks.remove_exited(&current_task)
    };

    // This is put after `current_thread.exit()`,
    // so `attach_tracee` will observe that the tracer has exited while
    // holding the `tracees` lock, and can not race with `clear_tracees`.
    posix_thread.clear_tracees();

    if let Some(tracer) = posix_thread.tracer() {
        let tracer = tracer.as_posix_thread().unwrap();
        tracer.process().children_wait_queue().wake_all();
    }

    // Release the listener before waking a joiner, and retain zombie status.
    // The task has already been removed from the set considered by TSYNC.
    posix_thread.set_seccomp(posix_thread.seccomp_snapshot().exited());
    wake_clear_ctid(thread_local);

    wake_robust_list(thread_local, posix_thread.tid());

    // According to Linux behavior, the main thread shouldn't be removed from the table until the
    // process is reaped by its parent.
    if posix_thread.tid() != posix_process.pid() {
        pid_table::pid_table_mut().remove_thread(posix_thread.tid());
    }

    // Drop fields in `PosixThread`.
    drop_after!(posix_thread.file_table().lock().take());
    drop_after!(posix_thread.fs.write().take());
    drop_after!(posix_thread.ns_proxy().lock().take());

    // Drop fields in `ThreadLocal`.
    drop_after!(thread_local.vmar().borrow_mut().take());
    drop_after!(thread_local.remove_file_table());
    drop_after!(thread_local.remove_fs());
    drop_after!(thread_local.remove_ns_proxy());

    if is_last_thread {
        exit_process(&posix_process);
    }
}

/// Sends `SIGKILL` to all other threads in the current process.
pub(in crate::process) fn sigkill_other_threads(current_task: &Task, task_set: &TaskSet) {
    debug_assert!(
        task_set
            .as_slice()
            .iter()
            .any(|task| core::ptr::eq(current_task, task.as_ref()))
    );

    for task in task_set.as_slice() {
        if core::ptr::eq(current_task, task.as_ref()) {
            continue;
        }
        task.as_posix_thread()
            .unwrap()
            .enqueue_signal(Box::new(KernelSignal::new(SIGKILL)));
    }
}

/// Writes zero to `clear_child_tid` and performs a futex wake.
fn wake_clear_ctid(thread_local: &ThreadLocal) {
    let clear_ctid = thread_local.clear_child_tid().get();

    if clear_ctid == 0 {
        return;
    }

    let _ = current_userspace!()
        .write_val(clear_ctid, &0u32)
        .inspect_err(|err| debug!("exit: cannot clear the child TID: {:?}", err));
    let _ = futex_wake(clear_ctid, 1, FutexVisibility::Shared)
        .inspect_err(|err| debug!("exit: cannot wake the futex on the child TID: {:?}", err));

    thread_local.clear_child_tid().set(0);
}

/// Walks the robust futex list, marking futex dead and waking waiters.
///
/// This corresponds to Linux's `exit_robust_list`. Errors are silently ignored.
fn wake_robust_list(thread_local: &ThreadLocal, tid: Tid) {
    let list_head = match thread_local.robust_list().borrow_mut().take() {
        Some(robust_list_head) => robust_list_head,
        None => return,
    };

    debug!("exit: wake up the robust list: {:?}", list_head);
    for futex_addr in list_head.futexes() {
        if let Err(err) = wake_robust_futex(futex_addr, tid) {
            debug!(
                "exit: cannot wake the robust futex at {:#x}: {:?}",
                futex_addr, err
            );
            return;
        }
    }
}
