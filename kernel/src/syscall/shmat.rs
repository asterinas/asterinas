// SPDX-License-Identifier: MPL-2.0

//! This mod defines the handler to syscall shmat

use align_ext::AlignExt;

use super::SyscallReturn;
use crate::{
    fs::utils::InodeMode,
    prelude::*,
    vm::{
        perms::VmPerms,
        shared_mem::{SHM_OBJ_MANAGER, SHMLBA},
        vmar::is_userspace_vaddr,
    },
};

bitflags! {
    /// Flags for `shmat()` (shared memory attach) operations.
    pub struct ShmFlags: u32 {
        /// Read-only access (equivalent to `SHM_RDONLY`).
        const RDONLY = 0o10000;
        /// Round attach address to SHMLBA boundary (equivalent to `SHM_RND`).
        const RND    = 0o20000;
        /// Remap existing mapping (equivalent to `SHM_REMAP`).
        const REMAP  = 0o40000;
        /// Execution access (equivalent to `SHM_EXEC`).
        const EXEC   = 0o100000;
    }
}

pub fn sys_shmat(shmid: i32, addr: u64, flags: i32, ctx: &Context) -> Result<SyscallReturn> {
}
