// SPDX-License-Identifier: MPL-2.0

//! This mod defines the handler to syscall shmget

use super::SyscallReturn;
use crate::{
    fs::utils::InodeMode,
    ipc::IpcFlags,
    prelude::*,
    vm::shared_mem::{SHMMAX, SHMMIN, SHM_OBJ_MANAGER},
};

pub fn sys_shmget(key: i32, size: usize, flags: i32, ctx: &Context) -> Result<SyscallReturn> {
    const IPC_PRIVATE: i32 = 0;
    if key < 0 {
        return_errno!(Errno::EINVAL);
    }
    if size <= SHMMIN || size > SHMMAX {
        return_errno!(Errno::EINVAL);
    }

    let mode = InodeMode::from_bits_truncate((flags & 0o777) as u16);
    let flags = IpcFlags::from_bits_truncate(flags as u32);

    debug!(
        "[sys_shmget] key = {}, size = {}, flags = {:?}",
        key, size, flags
    );
    let uid = ctx.process.pid();
    let gid = ctx.process.pgid();

    let manager = SHM_OBJ_MANAGER.get().ok_or(Errno::EINVAL)?;

    let shmid = if key == IPC_PRIVATE {
        // If key is IPC_PRIVATE, create an anonymous shared memory segment
        manager.create_shm_anonymous(size, mode, uid)?
    } else {
        let shm_exists = manager.shm_exists(key as u32);
        let shm_key = key as u32;

        if flags.contains(IpcFlags::IPC_CREAT) {
            if shm_exists {
                if flags.contains(IpcFlags::IPC_EXCL) {
                    return_errno!(Errno::EEXIST);
                }
                manager.get_shmid_by_key(shm_key, uid, gid)?
            } else {
                manager.create_shm(shm_key, size, mode, uid)?
            }
        } else if shm_exists {
            manager.get_shmid_by_key(shm_key, uid, gid)?
        } else {
            // If IPC_CREAT is not set, the segment must exist
            return_errno!(Errno::ENOENT);
        }
    };

    Ok(SyscallReturn::Return(shmid as _))
}
