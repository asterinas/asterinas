// SPDX-License-Identifier: MPL-2.0

//! This mod defines the handler to syscall shmdt

use align_ext::AlignExt;

use super::SyscallReturn;
use crate::{prelude::*, vm::shared_mem::SHM_OBJ_MANAGER};

pub fn sys_shmdt(addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
}
