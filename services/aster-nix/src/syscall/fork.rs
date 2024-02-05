// SPDX-License-Identifier: MPL-2.0

use crate::{
    log_syscall_entry,
    prelude::*,
    process::{clone_child, CloneArgs},
};
use aster_frame::cpu::UserContext;

use crate::syscall::SYS_FORK;

use super::SyscallReturn;

pub fn sys_fork(parent_context: UserContext) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_FORK);
    let current = current!();
    // FIXME: set correct args for fork
    let clone_args = CloneArgs::default();
    let child_pid = clone_child(parent_context, clone_args).unwrap();
    Ok(SyscallReturn::Return(child_pid as _))
}
