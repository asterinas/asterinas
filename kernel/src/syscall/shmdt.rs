// SPDX-License-Identifier: MPL-2.0

//! This mod defines the handler to syscall shmdt

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_shmdt(addr: u64, ctx: &Context) -> Result<SyscallReturn> {
    todo!("Implement sys_shmdt for shmdt syscall");
}
