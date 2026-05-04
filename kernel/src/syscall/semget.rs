// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{ipc::IpcFlags, prelude::*};

pub fn sys_semget(key: i32, nsems: i32, semflg: i32, ctx: &Context) -> Result<SyscallReturn> {
    let num_sems = nsems as usize;
    let flags = IpcFlags::from_bits_truncate(semflg.cast_unsigned());
    let mode: u16 = (semflg.cast_unsigned() & 0x1FF) as u16;

    debug!(
        "semget: key = {}, num_sems = {}, flags = {:?}, mode = {:03o}",
        key, num_sems, flags, mode
    );

    let ns_proxy = ctx.thread_local.borrow_ns_proxy();
    let ipc_ns = ns_proxy.unwrap().ipc_ns();

    let credentials = ctx.posix_thread.credentials();
    let semid = ipc_ns.get_or_create_sem_set(key, num_sems, flags, mode, credentials)?;

    Ok(SyscallReturn::Return(semid.get() as isize))
}
