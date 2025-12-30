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
    debug!("[sys_shmctl] id = {}, cmd = {}, buf = {:#x}", id, cmd, buf);

    if id < 0 {
        return_errno!(Errno::EINVAL);
    }

    let manager = SHM_OBJ_MANAGER.get().ok_or(Errno::EINVAL)?;
    let shm_obj = {
        let guard = manager.read();
        match guard.get_shm_obj(id as u64) {
            Some(shm_obj) => shm_obj,
            None => return_errno!(Errno::EINVAL),
        }
    };

    let cmd = ShmCtlCmd::from_bits(cmd).ok_or(Errno::EINVAL)?;

    match cmd {
        ShmCtlCmd::IPC_RMID => {
            let mut guard = manager.write();

            // Remove key mapping immediately to block further lookups by key
            if let Some(key) = shm_obj.key() {
                guard.remove_key_mapping(key);
            }

            // Mark the segment to be destroyed
            shm_obj.set_deleted();

            // Try to delete the shared memory object when no process is attached
            guard.try_delete_shm_obj(id as u64)?;

            Ok(SyscallReturn::Return(0))
        }
        ShmCtlCmd::IPC_SET => {
            let shm_ds: ShmidDs = current_userspace!().read_val(buf as usize)?;

            // FIXME: Check if the current process has the permission to set
            // the attributes

            // Update the attributes of the shared memory object
            shm_obj.set_attributes(
                InodeMode::from_bits_truncate(shm_ds.shm_perm.mode),
                shm_ds.shm_perm.uid,
                shm_ds.shm_perm.gid,
            )?;

            Ok(SyscallReturn::Return(0))
        }
        ShmCtlCmd::IPC_STAT => {
            // Get the attributes of the shared memory object
            let shm_ds = shm_obj.get_attributes()?;

            // Write the attributes to the user space
            current_userspace!().write_val(buf as usize, &shm_ds)?;

            Ok(SyscallReturn::Return(0))
        }
        _ => {
            warn!("Unsupported shmctl command: {:?}", cmd);
            return_errno!(Errno::EINVAL);
        }
    }
}
