// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    ipc::{
        sem::{sem_op, SemBuf},
        sem_set::SEMOPM,
    },
    prelude::*,
    time::timespec_t,
    util::read_val_from_user,
};

pub fn sys_semop(sem_id: i32, tsops: Vaddr, nsops: usize) -> Result<SyscallReturn> {
    debug!(
        "[sys_semop] sem_id:{:?}, tsops_vaddr:{:x?}, nsops:{:?}",
        sem_id, tsops, nsops
    );
    do_sys_semtimedop(sem_id, tsops, nsops, None)
}

pub fn sys_semtimedop(
    sem_id: i32,
    tsops: Vaddr,
    nsops: usize,
    timeout: Vaddr,
) -> Result<SyscallReturn> {
    debug!(
        "[sys_semtimedop] sem_id:{:?}, tsops_vaddr:{:x?}, nsops:{:?}, timeout_vaddr:{:x?}",
        sem_id, tsops, nsops, timeout
    );

    let timeout = if timeout == 0 {
        None
    } else {
        Some(read_val_from_user::<timespec_t>(timeout)?)
    };

    do_sys_semtimedop(sem_id, tsops, nsops, timeout)
}

fn do_sys_semtimedop(
    sem_id: i32,
    tsops: Vaddr,
    nsops: usize,
    timeout: Option<timespec_t>,
) -> Result<SyscallReturn> {
    if sem_id <= 0 || nsops == 0 {
        return_errno!(Errno::EINVAL);
    }
    if nsops > SEMOPM {
        return_errno!(Errno::E2BIG);
    }

    if let Some(timeout) = timeout.as_ref() {
        if timeout.sec < 0 || timeout.nsec < 0 || timeout.nsec >= 1_000_000_000 {
            return_errno!(Errno::EINVAL);
        }
    }

    for i in 0..nsops {
        let sem_buf = read_val_from_user::<SemBuf>(tsops + size_of::<SemBuf>() * i)?;
        sem_op(sem_id, sem_buf, timeout)?;
    }

    Ok(SyscallReturn::Return(0))
}
