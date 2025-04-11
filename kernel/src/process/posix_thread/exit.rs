// SPDX-License-Identifier: MPL-2.0

use ostd::task::{CurrentTask, Task};

use super::{
    futex::futex_wake, robust_list::wake_robust_futex, thread_table, AsPosixThread, AsThreadLocal,
    ThreadLocal,
};
use crate::{
    current_userspace,
    prelude::*,
    process::{
        exit::exit_process,
        signal::{constants::SIGKILL, signals::kernel::KernelSignal},
        task_set::TaskSet,
        TermStatus,
    },
    thread::{AsThread, Tid},
};

/// Exits the current POSIX thread.
///
/// # Panics
///
/// If the current thread is not a POSIX thread, this method will panic.
pub fn do_exit(term_status: TermStatus) {
    exit_internal(term_status, false);
}

/// Kills all threads and exits the current POSIX process.
///
/// # Panics
///
/// If the current thread is not a POSIX thread, this method will panic.
pub fn do_exit_group(term_status: TermStatus) {
    exit_internal(term_status, true);
}

/// Exits the current POSIX thread or process.
fn exit_internal(term_status: TermStatus, is_exiting_group: bool) {
    let current_task = Task::current().unwrap();
    let current_thread = current_task.as_thread().unwrap();
    let posix_thread = current_thread.as_posix_thread().unwrap();
    let thread_local = current_task.as_thread_local().unwrap();
    let posix_process = posix_thread.process();

    let is_last_thread = {
        let mut tasks = posix_process.tasks().lock();
        let has_exited_group = tasks.has_exited_group();

        if is_exiting_group && !has_exited_group {
            sigkill_other_threads(&current_task, &tasks);
            tasks.set_exited_group();
        }

        // According to Linux's behavior, the last thread's exit code will become the process's
        // exit code, so here we should just overwrite the old value (if any).
        if !has_exited_group {
            posix_process.status().set_exit_code(term_status.as_u32());
        }

        // We should only change the thread status when running as the thread, so no race
        // conditions can occur in between.
        if current_thread.is_exited() {
            return;
        }
        current_thread.exit();

        tasks.remove_exited(&current_task)
    };

    wake_clear_ctid(thread_local);

    wake_robust_list(thread_local, posix_thread.tid());

    // According to Linux behavior, the main thread shouldn't be removed from the table until the
    // process is reaped by its parent.
    if posix_thread.tid() != posix_process.pid() {
        thread_table::remove_thread(posix_thread.tid());
    }

    // Drop fields in `PosixThread`.
    *posix_thread.file_table().lock() = None;

    // Drop fields in `ThreadLocal`.
    *thread_local.root_vmar().borrow_mut() = None;
    thread_local.borrow_file_table_mut().remove();

    if is_last_thread {
        exit_process(&posix_process);
    }
}

/// Sends `SIGKILL` to all other threads in the current process.
///
/// This is only needed when initiating an `exit_group` for the first time.
fn sigkill_other_threads(current_task: &CurrentTask, task_set: &TaskSet) {
    for task in task_set.as_slice() {
        if core::ptr::eq(current_task.as_ref(), task.as_ref()) {
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
    let _ = futex_wake(clear_ctid, 1, None)
        .inspect_err(|err| debug!("exit: cannot wake the futex on the child TID: {:?}", err));

    thread_local.clear_child_tid().set(0);
}

/// Walks the robust futex list, marking futex dead and waking waiters.
///
/// This corresponds to Linux's `exit_robust_list`. Errors are silently ignored.
fn wake_robust_list(thread_local: &ThreadLocal, tid: Tid) {
    let mut robust_list = thread_local.robust_list().borrow_mut();

    let list_head = match *robust_list {
        Some(robust_list_head) => robust_list_head,
        None => return,
    };

    trace!("exit: wake up the rubust list: {:?}", list_head);
    for futex_addr in list_head.futexes() {
        let _ = wake_robust_futex(futex_addr, tid)
            .inspect_err(|err| debug!("exit: cannot wake up the robust futex: {:?}", err));
    }

    *robust_list = None;
}
