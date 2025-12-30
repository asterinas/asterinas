// SPDX-License-Identifier: MPL-2.0

//! This mod defines the handler to syscall shmget

use super::SyscallReturn;
use crate::{
    fs::utils::InodeMode,
    ipc::IpcFlags,
    prelude::*,
    process::{Gid, Uid},
    vm::shared_mem::{SHM_OBJ_MANAGER, SHMMAX, SHMMIN},
};

pub fn sys_shmget(key: i32, size: usize, flags: i32, ctx: &Context) -> Result<SyscallReturn> {
}
