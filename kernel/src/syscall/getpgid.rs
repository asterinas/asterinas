// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{process_table, Pid},
};

pub fn sys_getpgid(pid: Pid, ctx: &Context) -> Result<SyscallReturn> {
    debug!("pid = {}", pid);
    // type Pid = u32, pid would never less than 0.
    // if pid < 0 {
    //     return_errno_with_message!(Errno::EINVAL, "pid cannot be negative");
    // }

    // if pid is 0, should return the pgid of current process
    if pid == 0 {
        return Ok(SyscallReturn::Return(ctx.process.pgid() as _));
    }

    let process = process_table::get_process(pid)
        .ok_or(Error::with_message(Errno::ESRCH, "process does not exist"))?;

    if !Arc::ptr_eq(&ctx.process.session().unwrap(), &process.session().unwrap()) {
        return_errno_with_message!(
            Errno::EPERM,
            "the process and current process does not belong to the same session"
        );
    }

    Ok(SyscallReturn::Return(process.pgid() as _))
}
