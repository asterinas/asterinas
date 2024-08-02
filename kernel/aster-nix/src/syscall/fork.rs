// SPDX-License-Identifier: MPL-2.0

use ostd::cpu::UserContext;

use super::{CallingThreadInfo, SyscallReturn};
use crate::{
    prelude::*,
    process::{clone_child, CloneArgs},
};

pub fn sys_fork(
    parent_info: CallingThreadInfo,
    parent_context: &UserContext,
) -> Result<SyscallReturn> {
    let clone_args = CloneArgs::for_fork();
    let child_pid = clone_child(parent_info, parent_context, clone_args).unwrap();
    Ok(SyscallReturn::Return(child_pid as _))
}
