// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    ipc::{
        semaphore::system_v::{
            sem_set::{check_sem, create_sem_set, create_sem_set_with_id, SEMMSL},
            PermissionMode,
        },
        IpcFlags,
    },
    prelude::*,
};

pub fn sys_semget(key: i32, nsems: i32, semflags: i32, ctx: &Context) -> Result<SyscallReturn> {
    if nsems < 0 || nsems as usize > SEMMSL {
        return_errno!(Errno::EINVAL);
    }
    if key < 0 {
        return_errno!(Errno::EINVAL);
    }

    let flags = IpcFlags::from_bits_truncate(semflags as u32);
    let mode: u16 = (semflags as u32 & 0x1FF) as u16;
    let nsems = nsems as usize;
    let credentials = ctx.posix_thread.credentials();

    debug!(
        "[sys_semget] key = {}, nsems = {}, flags = {:?}",
        key, nsems, semflags
    );

    // Create a new semaphore set directly
    const IPC_NEW: i32 = 0;
    if key == IPC_NEW {
        if nsems == 0 {
            return_errno!(Errno::EINVAL);
        }
        return Ok(SyscallReturn::Return(
            create_sem_set(nsems, mode, credentials)? as isize,
        ));
    }

    // Get a semaphore set, and create if necessary
    match check_sem(
        key,
        Some(nsems),
        PermissionMode::ALTER | PermissionMode::READ,
    ) {
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
            if nsems == 0 {
                return_errno!(Errno::EINVAL);
            }

            create_sem_set_with_id(key, nsems, mode, credentials)?
        }
    };

    Ok(SyscallReturn::Return(key as isize))
}
