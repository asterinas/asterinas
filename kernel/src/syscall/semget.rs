// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    ipc::{IpcFlags, semaphore::system_v::sem_set::SEMMSL},
    prelude::*,
};

pub fn sys_semget(key: i32, num_sems: i32, semflags: i32, ctx: &Context) -> Result<SyscallReturn> {
    if num_sems < 0 || num_sems as usize > SEMMSL {
        return_errno_with_message!(Errno::EINVAL, "invalid num_sems value");
    }
    if key < 0 {
        return_errno_with_message!(Errno::EINVAL, "semaphore key must not be negative");
    }

    let flags = IpcFlags::from_bits_truncate(semflags as u32);
    let mode: u16 = (semflags as u32 & 0x1FF) as u16;
    let num_sems = num_sems as usize;
    let credentials = ctx.posix_thread.credentials();

    debug!(
        "[sys_semget] key = {}, num_sems = {}, flags = {:?}",
        key, num_sems, semflags
    );

    let ns_proxy = ctx.thread_local.borrow_ns_proxy();
    let ipc_ns = ns_proxy.unwrap().ipc_ns();

    Ok(SyscallReturn::Return(ipc_ns.get_or_create_sem_set(
        key,
        num_sems,
        flags,
        mode,
        credentials,
    )? as isize))
}
