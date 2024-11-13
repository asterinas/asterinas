// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use super::SyscallReturn;
use crate::{
    ipc::semaphore::system_v::{
        sem::{sem_op, SemBuf},
        sem_set::SEMOPM,
    },
    prelude::*,
    time::timespec_t,
};

pub fn sys_semop(sem_id: i32, tsops: Vaddr, nsops: usize, ctx: &Context) -> Result<SyscallReturn> {
    debug!(
        "[sys_semop] sem_id = {:?}, tsops_vaddr = {:x?}, nsops = {:?}",
        sem_id, tsops, nsops
    );
    do_sys_semtimedop(sem_id, tsops, nsops, None, ctx)
}

pub fn sys_semtimedop(
    sem_id: i32,
    tsops: Vaddr,
    nsops: usize,
    timeout: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "[sys_semtimedop] sem_id = {:?}, tsops_vaddr = {:x?}, nsops = {:?}, timeout_vaddr = {:x?}",
        sem_id, tsops, nsops, timeout
    );

    let timeout = if timeout == 0 {
        None
    } else {
        Some(Duration::try_from(
            ctx.user_space().read_val::<timespec_t>(timeout)?,
        )?)
    };

    do_sys_semtimedop(sem_id, tsops, nsops, timeout, ctx)
}

fn do_sys_semtimedop(
    sem_id: i32,
    tsops: Vaddr,
    nsops: usize,
    timeout: Option<Duration>,
    ctx: &Context,
) -> Result<SyscallReturn> {
    if sem_id <= 0 || nsops == 0 {
        return_errno!(Errno::EINVAL);
    }
    if nsops > SEMOPM {
        return_errno!(Errno::E2BIG);
    }

    let user_space = ctx.user_space();
    let mut semops = Vec::with_capacity(nsops);
    for i in 0..nsops {
        semops.push(user_space.read_val::<SemBuf>(tsops + size_of::<SemBuf>() * i)?);
    }

    sem_op(sem_id, semops, timeout, ctx)?;

    Ok(SyscallReturn::Return(0))
}
