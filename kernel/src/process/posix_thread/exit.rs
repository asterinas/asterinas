// SPDX-License-Identifier: MPL-2.0

use super::{futex::futex_wake, robust_list::wake_robust_futex, thread_table, PosixThread};
use crate::{
    current_userspace,
    prelude::*,
    process::{do_exit_group, TermStatus},
    thread::{Thread, Tid},
};

/// Exits the thread if the thread is a POSIX thread.
///
/// # Panics
///
/// If the thread is not a POSIX thread, this method will panic.
pub fn do_exit(thread: &Thread, posix_thread: &PosixThread, term_status: TermStatus) -> Result<()> {
    if thread.is_exited() {
        return Ok(());
    }
    thread.exit();

    let tid = posix_thread.tid;

    let mut clear_ctid = posix_thread.clear_child_tid().lock();
    // If clear_ctid !=0 ,do a futex wake and write zero to the clear_ctid addr.
    if *clear_ctid != 0 {
        // FIXME: the correct write length?
        if let Err(e) = current_userspace!().write_val(*clear_ctid, &0u32) {
            debug!("Ignore error during exit process: {:?}", e);
        }
        futex_wake(*clear_ctid, 1, None)?;
        *clear_ctid = 0;
    }
    drop(clear_ctid);
    // exit the robust list: walk the robust list; mark futex words as dead and do futex wake
    wake_robust_list(posix_thread, tid);

    if tid != posix_thread.process().pid() {
        // We don't remove main thread.
        // The main thread is removed when the process is reaped.
        thread_table::remove_thread(tid);
    }

    if posix_thread.is_main_thread(tid) || posix_thread.is_last_thread() {
        // exit current process.
        do_exit_group(term_status);
    }

    futex_wake(Arc::as_ptr(&posix_thread.process()) as Vaddr, 1, None)?;
    Ok(())
}

/// Walks the robust futex list, marking futex dead and wake waiters.
/// It corresponds to Linux's exit_robust_list(), errors are silently ignored.
fn wake_robust_list(thread: &PosixThread, tid: Tid) {
    let mut robust_list = thread.robust_list.lock();
    let list_head = match *robust_list {
        Some(robust_list_head) => robust_list_head,
        None => return,
    };
    trace!("wake the rubust_list: {:?}", list_head);
    for futex_addr in list_head.futexes() {
        wake_robust_futex(futex_addr, tid).unwrap();
    }
    *robust_list = None;
}
