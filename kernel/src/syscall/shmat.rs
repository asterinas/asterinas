// SPDX-License-Identifier: MPL-2.0

//! This mod defines the handler to syscall shmat

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_shmat(shmid: i32, addr: u64, flags: i32, ctx: &Context) -> Result<SyscallReturn> {
    todo!("Implement sys_shmat for shmat syscall");
}
