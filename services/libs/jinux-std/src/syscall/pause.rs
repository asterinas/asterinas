use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::signal::sig_mask::SigMask;
use crate::process::signal::SigQueueObserver;

use super::{SyscallReturn, SYS_PAUSE};

pub fn sys_pause() -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_PAUSE);

    let sigqueue_observer = {
        // FIXME: like sleep, paused thread can only be interrupted by signals that will call signal
        // handler or terminate current process
        let sigmask = SigMask::new_full();
        SigQueueObserver::new(sigmask)
    };

    sigqueue_observer.wait_until_interruptible(|| None, None)?;

    unreachable!("[Internal Error] pause should always return EINTR");
}
