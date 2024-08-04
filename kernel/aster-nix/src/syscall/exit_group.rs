// SPDX-License-Identifier: MPL-2.0

use crate::{
    prelude::*,
    process::{do_exit_group, TermStatus},
    syscall::SyscallReturn,
};

/// Exit all thread in a process.
pub fn sys_exit_group(exit_code: u64, _ctx: &Context) -> Result<SyscallReturn> {
    // Exit all thread in current process
    let term_status = TermStatus::Exited(exit_code as _);
    do_exit_group(term_status);
    Ok(SyscallReturn::Return(0))
}
