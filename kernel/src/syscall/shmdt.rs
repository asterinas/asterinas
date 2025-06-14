// SPDX-License-Identifier: MPL-2.0

//! This mod defines the handler to syscall shmdt

use align_ext::AlignExt;

use super::SyscallReturn;
use crate::{prelude::*, vm::shared_mem::SHM_OBJ_MANAGER};

pub fn sys_shmdt(addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    debug!("[sys_shmdt] addr = {:#x}", addr);

    if addr % PAGE_SIZE != 0 {
        return_errno_with_message!(Errno::EINVAL, "shmdt addr must be page-aligned");
    }

    let user_space = ctx.user_space();
    let root_vmar = user_space.root_vmar();

    let shmid: u64 = root_vmar.get_shm_id(addr)?;
    let manager = SHM_OBJ_MANAGER.get().ok_or(Errno::EINVAL)?;
    let shm_obj = match manager.get_shm_obj(shmid) {
        Some(shm_obj) => shm_obj,
        None => return_errno!(Errno::EINVAL),
    };

    // Remove the mapping from the VMAR
    let len = (shm_obj.size()).align_up(PAGE_SIZE);
    let end = addr.checked_add(len).ok_or(Error::with_message(
        Errno::EINVAL,
        "integer overflow when (addr + len)",
    ))?;
    debug!("shmdt range = 0x{:x} - 0x{:x}", addr, end);
    root_vmar.remove_mapping(addr..end)?;

    // Decrease the reference count of the shared memory object
    shm_obj.set_detached(ctx.process.pid());
    if shm_obj.should_be_deleted() {
        manager.try_delete_shm_obj(shmid)?;
    }

    Ok(SyscallReturn::Return(0))
}
