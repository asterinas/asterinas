// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{process_table, Pid},
};

pub fn sys_getpgid(pid: Pid, ctx: &Context) -> Result<SyscallReturn> {
    debug!("pid = {}", pid);

    // The documentation quoted below is from
    // <https://www.man7.org/linux/man-pages/man2/getpgid.2.html>.

    // "If `pid` is equal to 0, getpgid() shall return the process group ID of the calling
    // process."
    if pid == 0 {
        return Ok(SyscallReturn::Return(ctx.process.pgid() as _));
    }

    let process = process_table::get_process(pid).ok_or(Error::with_message(
        Errno::ESRCH,
        "the process to get the PGID does not exist",
    ))?;

    // The man pages allow the implementation to return `EPERM` if `process` is in a different
    // session than the current process. Linux does not perform this check by default, but some
    // strict security policies (e.g. SELinux) may do so.

    Ok(SyscallReturn::Return(process.pgid() as _))
}
