// SPDX-License-Identifier: MPL-2.0

//! This mod defines the handler to syscall shmdt

use align_ext::AlignExt;

use super::SyscallReturn;
use crate::{prelude::*, vm::shared_mem::SHM_OBJ_MANAGER};

pub fn sys_shmdt(addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    debug!("[sys_shmdt] addr = {:#x}", addr);

    if !addr.is_multiple_of(PAGE_SIZE) {
        return_errno_with_message!(Errno::EINVAL, "shmdt addr must be page-aligned");
    }

    let user_space = ctx.user_space();
    let root_vmar = user_space.vmar();

    let shmid: u64 = root_vmar.get_shm_id(addr)?;
    let manager = SHM_OBJ_MANAGER.get().ok_or(Error::new(Errno::EINVAL))?;
    let shm_obj = {
        let manager_guard = manager.read();
        manager_guard
            .get_shm_obj(shmid)
            .ok_or(Error::new(Errno::EINVAL))?
    };
    let shm_size = shm_obj.size().align_up(PAGE_SIZE);

    // Find the minimal start address among all mappings of this shmid.
    //
    // Since it is allowed that `munmap` partial mappings of a shared memory
    // segment, there might be multiple mappings for the same shared memory
    // segment. Only allow detaching when the addr is the minimal start address
    // among all mappings of this shmid.
    let min_start = root_vmar.min_shared_mapping_start(shmid)?;
    if addr != min_start {
        return_errno!(Errno::EINVAL);
    }

    // Remove the mapping range of the shared segment starting from base.
    let end = addr
        .checked_add(shm_size)
        .ok_or(Error::with_message(Errno::EINVAL, "overflow in shmdt len"))?;
    debug!("shmdt range = 0x{:x} - 0x{:x}", addr, end);
    root_vmar.remove_mapping(addr..end)?;

    Ok(SyscallReturn::Return(0))
}
