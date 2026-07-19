// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    ipc::{
        IpcId,
        semaphore::system_v::{
            sem::{SemBuf, sem_op},
            sem_set::SEMOPM,
        },
    },
    prelude::*,
    time::timespec_t,
};

pub fn sys_semop(semid: i32, sops: Vaddr, nsops: usize, ctx: &Context) -> Result<SyscallReturn> {
    do_sys_semtimedop(semid, sops, nsops, None, ctx)
}

pub fn sys_semtimedop(
    semid: i32,
    sops: Vaddr,
    nsops: usize,
    timeout: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let timeout = if timeout == 0 {
        None
    } else {
        Some(Duration::try_from(
            ctx.user_space().read_val::<timespec_t>(timeout)?,
        )?)
    };

    do_sys_semtimedop(semid, sops, nsops, timeout, ctx)
}

fn do_sys_semtimedop(
    semid: i32,
    sops: Vaddr,
    nsops: usize,
    timeout: Option<Duration>,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "semop: semid = {:?}, sops = {:#x}, nsops = {:?}, timeout = {:?}",
        semid, sops, nsops, timeout
    );

    let Ok(semid) = IpcId::try_from(semid.cast_unsigned()) else {
        return_errno_with_message!(Errno::EINVAL, "non-positive semaphore IDs are invalid");
    };

    if nsops == 0 {
        return_errno_with_message!(Errno::EINVAL, "the number of operations is zero");
    }
    if nsops > SEMOPM {
        return_errno_with_message!(Errno::E2BIG, "the number of operations exceeds SEMOPM");
    }

    let user_space = ctx.user_space();
    let mut semops = Vec::with_capacity(nsops);
    for i in 0..nsops {
        semops.push(user_space.read_val::<SemBuf>(sops + size_of::<SemBuf>() * i)?);
    }

    let ns_proxy = ctx.thread_local.borrow_ns_proxy();
    let ipc_ns = ns_proxy.unwrap().ipc_ns();

    sem_op(semid, semops, timeout, ipc_ns, ctx)?;

    Ok(SyscallReturn::Return(0))
}
