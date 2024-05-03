// SPDX-License-Identifier: MPL-2.0

use aster_frame::cpu::UserContext;

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{clone_child, CloneArgs},
};

pub fn sys_fork(parent_context: &UserContext) -> Result<SyscallReturn> {
    let current = current!();
    // FIXME: set correct args for fork
    let clone_args = CloneArgs::default();
    let child_pid = clone_child(*parent_context, clone_args).unwrap();
    Ok(SyscallReturn::Return(child_pid as _))
}
