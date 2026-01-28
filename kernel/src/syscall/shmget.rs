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
        manager.write().create_shm_anonymous(size, mode, cpid)?
    } else {
        let shm_key = key as u32;
        let shmid = manager.read().get_shmid_by_key(shm_key);

        if let Some(existing_shmid) = shmid {
            let shm_obj = manager
                .read()
                .get_shm_obj(existing_shmid)
                .ok_or(Errno::ENOENT)?;
            let mode = shm_obj.mode()?;

            if size > shm_obj.size() {
                return_errno!(Errno::EINVAL);
            }

            if u32::from(uid) == shm_obj.uid()? {
                if !mode.contains(InodeMode::S_IRUSR) {
                    return_errno!(Errno::EACCES);
                }
            } else if u32::from(gid) == shm_obj.gid()? {
                if !mode.contains(InodeMode::S_IRGRP) {
                    return_errno!(Errno::EACCES);
                }
            } else if !mode.contains(InodeMode::S_IROTH) {
                return_errno!(Errno::EACCES);
            }

            if flags.contains(IpcFlags::IPC_CREAT) && flags.contains(IpcFlags::IPC_EXCL) {
                // Setting both IPC_CREAT and IPC_EXCL means to create a new segment, thus fail when it already exists
                return_errno!(Errno::EEXIST);
            }

            existing_shmid
        } else if flags.contains(IpcFlags::IPC_CREAT) {
            // Create a new shared memory segment
            manager.write().create_shm(shm_key, size, mode, cpid)?
        } else {
            // If IPC_CREAT is not set, the segment must exist
            return_errno!(Errno::ENOENT);
        }
    };

    Ok(SyscallReturn::Return(shmid as _))
}
