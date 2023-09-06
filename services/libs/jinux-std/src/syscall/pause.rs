use crate::{prelude::*, process::posix_thread::PosixThreadExt, thread::Thread};

use super::SyscallReturn;

pub fn sys_pause() -> Result<SyscallReturn> {
    loop {
        let current_thread = current_thread!();
        // check sig_queue of current thread and process,
        // if there's any pending signal, break loop
        let posix_thread = current_thread.as_posix_thread().unwrap();
        if posix_thread.has_pending_signal() || current!().has_pending_signal() {
            break;
        }
        // there's no pending signal, yield execution
        // FIXME: set current thread interruptible here
        Thread::yield_now();
    }
    // handle signal before returning to user space
    return_errno_with_message!(Errno::ERESTART, "catch signal")
}
