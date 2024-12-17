// SPDX-License-Identifier: MPL-2.0

use crate::{
    prelude::*,
    process::{posix_thread::do_exit, TermStatus},
    syscall::SyscallReturn,
};

pub fn sys_exit(exit_code: i32, _ctx: &Context) -> Result<SyscallReturn> {
    debug!("exid code = {}", exit_code);

    let term_status = TermStatus::Exited(exit_code as _);
    do_exit(term_status);

    Ok(SyscallReturn::Return(0))
}
