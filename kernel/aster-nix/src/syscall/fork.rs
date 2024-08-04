// SPDX-License-Identifier: MPL-2.0

use ostd::cpu::UserContext;

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{clone_child, CloneArgs},
};

pub fn sys_fork(ctx: &Context, parent_context: &UserContext) -> Result<SyscallReturn> {
    let clone_args = CloneArgs::for_fork();
    let child_pid = clone_child(ctx, parent_context, clone_args).unwrap();
    Ok(SyscallReturn::Return(child_pid as _))
}
