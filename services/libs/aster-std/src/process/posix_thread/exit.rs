use crate::prelude::*;
use crate::process::{do_exit_group, TermStatus};
use crate::thread::{thread_table, Thread, Tid};
use crate::util::write_val_to_user;

use super::futex::futex_wake;
use super::robust_list::wake_robust_futex;
use super::{PosixThread, PosixThreadExt};

/// Exit the thread if the thread is a posix thread.
///
/// # Panic
///
/// If the thread is not a posix thread, this method will method.
pub fn do_exit(thread: Arc<Thread>, term_status: TermStatus) -> Result<()> {
    if thread.is_exited() {
        return Ok(());
    }
    thread.exit();

    let tid = thread.tid();

    let posix_thread = thread.as_posix_thread().unwrap();

    let mut clear_ctid = posix_thread.clear_child_tid().lock();
    // If clear_ctid !=0 ,do a futex wake and write zero to the clear_ctid addr.
    if *clear_ctid != 0 {
        futex_wake(*clear_ctid, 1)?;
        // FIXME: the correct write length?
        write_val_to_user(*clear_ctid, &0u32).unwrap();
        *clear_ctid = 0;
    }
    // exit the robust list: walk the robust list; mark futex words as dead and do futex wake
    wake_robust_list(posix_thread, tid);

    if tid != posix_thread.process().pid() {
        // If the thread is not main thread. We don't remove main thread.
        // Main thread are removed when the whole process is reaped.
        thread_table::remove_thread(tid);
    }

    if is_main_thread(tid, posix_thread) || is_last_thread(posix_thread) {
        // exit current process.
        do_exit_group(term_status);
    }

    futex_wake(Arc::as_ptr(&posix_thread.process()) as Vaddr, 1)?;
    Ok(())
}

fn is_main_thread(tid: Tid, posix_thread: &PosixThread) -> bool {
    let process = posix_thread.process();
    let pid = process.pid();
    tid == pid
}

fn is_last_thread(thread: &PosixThread) -> bool {
    let process = thread.process.upgrade().unwrap();
    let threads = process.threads().lock();
    threads
        .iter()
        .filter(|thread| !thread.status().lock().is_exited())
        .count()
        == 0
}

/// Walks the robust futex list, marking futex dead and wake waiters.
/// It corresponds to Linux's exit_robust_list(), errors are silently ignored.
fn wake_robust_list(thread: &PosixThread, tid: Tid) {
    let mut robust_list = thread.robust_list.lock();
    let list_head = match *robust_list {
        None => {
            return;
        }
        Some(robust_list_head) => robust_list_head,
    };
    trace!("wake the rubust_list: {:?}", list_head);
    for futex_addr in list_head.futexes() {
        // debug!("futex addr = 0x{:x}", futex_addr);
        wake_robust_futex(futex_addr, tid).unwrap();
    }
    *robust_list = None;
}
