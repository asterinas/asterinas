// SPDX-License-Identifier: MPL-2.0

//! This mod defines the handler to syscall shmctl

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_shmctl(id: i32, cmd: i32, buf: u64, ctx: &Context) -> Result<SyscallReturn> {
    todo!("Implement sys_read for shmctl syscall");
}
