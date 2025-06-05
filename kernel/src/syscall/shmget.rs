// SPDX-License-Identifier: MPL-2.0

//! This mod defines the handler to syscall shmget

use super::SyscallReturn;
use crate::{
    fs::utils::InodeMode,
    ipc::IpcFlags,
    prelude::*,
    process::{Gid, Uid},
    vm::shared_mem::{SHMMAX, SHMMIN, SHM_OBJ_MANAGER},
};

pub fn sys_shmget(key: i32, size: usize, flags: i32, ctx: &Context) -> Result<SyscallReturn> {
    const IPC_PRIVATE: i32 = 0;
    const INODE_MODE_MASK: i32 = 0o777;
    if key < 0 {
        return_errno!(Errno::EINVAL);
    }
    if size <= SHMMIN || size > SHMMAX {
        return_errno!(Errno::EINVAL);
    }

    let mode = InodeMode::from_bits_truncate((flags & INODE_MODE_MASK) as u16);
    let flags = IpcFlags::from_bits_truncate(flags as u32);

    debug!(
        "[sys_shmget] key = {}, size = {}, flags = {:?}",
        key, size, flags
    );
    let uid = Uid::new_root();
    let gid = Gid::new_root();
    let cpid = ctx.process.pid();

    let manager = SHM_OBJ_MANAGER.get().ok_or(Errno::EINVAL)?;

    let shmid = if key == IPC_PRIVATE {
        // If key is IPC_PRIVATE, create an anonymous shared memory segment
        manager
            .write()
            .create_shm_anonymous(size, mode, cpid, uid, gid)?
    } else {
        let shm_exists = manager.read().shm_exists(key as u32);
        let shm_key = key as u32;

        if flags.contains(IpcFlags::IPC_CREAT) {
            if shm_exists {
                if flags.contains(IpcFlags::IPC_EXCL) {
                    return_errno!(Errno::EEXIST);
                }
                manager
                    .read()
                    .get_shmid_by_key(shm_key, uid.into(), gid.into())?
            } else {
                manager
                    .write()
                    .create_shm(shm_key, size, mode, cpid, uid, gid)?
            }
        } else if shm_exists {
            manager
                .read()
                .get_shmid_by_key(shm_key, uid.into(), gid.into())?
        } else {
            // If IPC_CREAT is not set, the segment must exist
            return_errno!(Errno::ENOENT);
        }
    };

    Ok(SyscallReturn::Return(shmid as _))
}
