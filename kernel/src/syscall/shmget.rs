// SPDX-License-Identifier: MPL-2.0

//! This mod defines the handler to syscall shmget

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_shmget(key: i32, size: usize, flags: i32, ctx: &Context) -> Result<SyscallReturn> {
    todo!("Implement sys_shmget for shmget syscall");
}
