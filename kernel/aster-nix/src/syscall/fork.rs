// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use ostd::cpu::UserContext;

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{clone_child, CloneArgs},
};

pub fn sys_fork(parent_context: &UserContext) -> Result<SyscallReturn> {
    let current = current!();
    let clone_args = CloneArgs::for_fork();
    let child_pid = clone_child(parent_context, clone_args).unwrap();
    Ok(SyscallReturn::Return(child_pid as _))
}
