// SPDX-License-Identifier: MPL-2.0

use super::{Thread, Tid};
use crate::{prelude::*, process::posix_thread::AsPosixThread};

static THREAD_TABLE: SpinLock<BTreeMap<Tid, Arc<Thread>>> = SpinLock::new(BTreeMap::new());

/// Adds a posix thread to global thread table
pub fn add_thread(tid: Tid, thread: Arc<Thread>) {
    debug_assert_eq!(tid, thread.as_posix_thread().unwrap().tid());
    THREAD_TABLE.lock().insert(tid, thread);
}

/// Removes a posix thread to global thread table
pub fn remove_thread(tid: Tid) {
    THREAD_TABLE.lock().remove(&tid);
}

/// Gets a posix thread from the global thread table
pub fn get_thread(tid: Tid) -> Option<Arc<Thread>> {
    THREAD_TABLE.lock().get(&tid).cloned()
}

/// Makes the current thread become the main thread if necessary.
pub(in crate::process) fn make_current_main_thread(ctx: &Context) {
    let pid = ctx.process.pid();
    let old_tid = ctx.posix_thread.tid();

    // The current thread is already the main thread.
    if old_tid == pid {
        return;
    }

    // The current thread is not the main thread.
    let mut tasks = ctx.process.tasks().lock();
    let mut thread_table = THREAD_TABLE.lock();

    assert!(tasks.has_exited_main());
    assert!(tasks.in_execve());
    assert_eq!(tasks.as_slice().len(), 2);
    assert!(core::ptr::eq(ctx.task, tasks.as_slice()[1].as_ref()));

    tasks.swap_main(pid, old_tid);
    ctx.posix_thread.set_main(pid);

    thread_table.remove(&pid).unwrap();
    let thread = thread_table.remove(&old_tid).unwrap();
    thread_table.insert(pid, thread);
}
