// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{process_table, Pid},
};

pub fn sys_getsid(pid: Pid, ctx: &Context) -> Result<SyscallReturn> {
    debug!("pid = {}", pid);

    // The documentation quoted below is from
    // <https://www.man7.org/linux/man-pages/man2/getsid.2.html>.

    // "If `pid` is 0, getsid() returns the session ID of the calling process."
    if pid == 0 {
        return Ok(SyscallReturn::Return(ctx.process.sid() as _));
    }

    let process = process_table::get_process(pid).ok_or(Error::with_message(
        Errno::ESRCH,
        "the process to get the SID does not exist",
    ))?;

    // The man pages allow the implementation to return `EPERM` if `process` is in a different
    // session than the current process. Linux does not perform this check by default, but some
    // strict security policies (e.g. SELinux) may do so.

    Ok(SyscallReturn::Return(process.sid() as _))
}
