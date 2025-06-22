// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{Pgid, Pid},
};

pub fn sys_setpgid(pid: Pid, pgid: Pgid, ctx: &Context) -> Result<SyscallReturn> {
    let current = ctx.process;

    // The documentation quoted below is from
    // <https://www.man7.org/linux/man-pages/man2/setpgid.2.html>.

    if pid.cast_signed() < 0 || pgid.cast_signed() < 0 {
        return_errno_with_message!(Errno::EINVAL, "negative PIDs or PGIDs are not valid");
    }

    // "If `pid` is zero, then the process ID of the calling process is used."
    let pid = if pid == 0 { current.pid() } else { pid };
    // "If `pgid` is zero, then the PGID of the process specified by `pid` is made the same as its
    // process ID."
    let pgid = if pgid == 0 { pid } else { pgid };

    debug!("pid = {}, pgid = {}", pid, pgid);

    current.move_process_to_group(pid, pgid)?;

    Ok(SyscallReturn::Return(0))
}
