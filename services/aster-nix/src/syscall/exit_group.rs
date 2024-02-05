// SPDX-License-Identifier: MPL-2.0

use crate::process::{do_exit_group, TermStatus};
use crate::{log_syscall_entry, prelude::*};

use crate::syscall::{SyscallReturn, SYS_EXIT_GROUP};

/// Exit all thread in a process.
pub fn sys_exit_group(exit_code: u64) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_EXIT_GROUP);
    // Exit all thread in current process
    let term_status = TermStatus::Exited(exit_code as _);
    do_exit_group(term_status);
    Ok(SyscallReturn::Return(0))
}
