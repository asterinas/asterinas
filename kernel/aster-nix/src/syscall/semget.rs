// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    ipc::{
        sem_set::{check_sem, create_sem_set, create_sem_set_with_id, SEMMSL},
        IpcFlags,
    },
    prelude::*,
};

pub fn sys_semget(key: i32, nsems: i32, semflags: i32) -> Result<SyscallReturn> {
    if nsems < 0 || nsems as usize > SEMMSL {
        return_errno!(Errno::EINVAL);
    }

    let flags = IpcFlags::from_bits_truncate(semflags as u32);
    let mode: u16 = (semflags as u32 & 0x1FF) as u16;
    let nsems = nsems as usize;

    debug!(
        "[sys_semget] key= {}, nsems = {}, flags = {:?}",
        key, nsems, semflags
    );

    // Create a new semaphore set directly
    const IPC_NEW: i32 = 0;
    if key == IPC_NEW {
        return Ok(SyscallReturn::Return(create_sem_set(nsems, mode)? as isize));
    }

    // Get a semaphore set, and create if necessary
    match check_sem(key, nsems) {
        Ok(_) => {
            if flags.contains(IpcFlags::IPC_CREAT | IpcFlags::IPC_EXCL) {
                return_errno!(Errno::EEXIST);
            }
        }
        Err(err) => {
            let need_create = err.error() == Errno::ENOENT && flags.contains(IpcFlags::IPC_CREAT);
            if !need_create {
                return Err(err);
            }
            create_sem_set_with_id(key, nsems, mode)?
        }
    };

    Ok(SyscallReturn::Return(key as isize))
}
