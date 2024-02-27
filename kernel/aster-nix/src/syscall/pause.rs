// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_PAUSE};
use crate::{log_syscall_entry, prelude::*, process::signal::Pauser};

pub fn sys_pause() -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_PAUSE);

    // FIXME: like sleep, paused thread can only be interrupted by signals that will call signal
    // handler or terminate current process
    let pauser = Pauser::new();

    pauser.pause_until(|| None)?;

    unreachable!("[Internal Error] pause should always return EINTR");
}
