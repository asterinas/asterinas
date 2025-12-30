// SPDX-License-Identifier: MPL-2.0

//! This mod defines the handler to syscall shmctl

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    current_userspace,
    fs::utils::InodeMode,
    prelude::*,
    vm::shared_mem::{SHM_OBJ_MANAGER, ShmidDs},
};

bitflags! {
    /// Commands for `shmctl()` operations.
    pub struct ShmCtlCmd: i32 {
        /// Remove the segment.
        const IPC_RMID = 0;
        /// Set segment information.
        const IPC_SET = 1;
        /// Get segment information.
        const IPC_STAT = 2;
        /// Lock segment in memory.
        const SHM_LOCK = 3;
        /// Unlock segment.
        const SHM_UNLOCK = 4;
        /// Get info about shared memory.
        const IPC_INFO = 5;
        /// Get shared memory info.
        const SHM_INFO = 6;
        /// Get statistics.
        const SHM_STAT = 7;
    }
}

pub fn sys_shmctl(id: i32, cmd: i32, buf: u64, _ctx: &Context) -> Result<SyscallReturn> {
}
