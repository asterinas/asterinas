// SPDX-License-Identifier: MPL-2.0

use crate::{
    prelude::*,
    process::{posix_thread::do_exit, TermStatus},
    syscall::SyscallReturn,
};

pub fn sys_exit(exit_code: i32) -> Result<SyscallReturn> {
    debug!("exid code = {}", exit_code);

    let current_thread = current_thread!();
    let term_status = TermStatus::Exited(exit_code as _);
    do_exit(current_thread, term_status)?;

    Ok(SyscallReturn::Return(0))
}
