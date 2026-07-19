// SPDX-License-Identifier: MPL-2.0

use ostd::arch::cpu::context::UserContext;

use crate::{
    prelude::*,
    process::{TermStatus, posix_thread::do_exit},
    syscall::SyscallReturn,
};

pub fn sys_exit(
    exit_code: i32,
    ctx: &Context,
    user_ctx: &mut UserContext,
) -> Result<SyscallReturn> {
    debug!("exid code = {}", exit_code);

    let term_status = TermStatus::Exited(exit_code as _);
    do_exit(term_status, ctx, user_ctx);

    Ok(SyscallReturn::Return(0))
}
