use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::{process_table, Pgid, Pid};

use super::{SyscallReturn, SYS_SETPGID};

pub fn sys_setpgid(pid: Pid, pgid: Pgid) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SETPGID);
    let current = current!();
    // if pid is 0, pid should be the pid of current process
    let pid = if pid == 0 { current.pid() } else { pid };
    // if pgid is 0, pgid should be pid
    let pgid = if pgid == 0 { pid } else { pgid };
    debug!("pid = {}, pgid = {}", pid, pgid);

    if pid != current.pid() && !current.has_child(&pid) {
        return_errno_with_message!(
            Errno::ESRCH,
            "cannot set pgid for process other than current or children of current"
        );
    }
    // FIXME: If pid is child process of current and already calls execve, should return error.
    // How can we determine a child process has called execve?

    // only can move process to an existing group or self
    if pgid != pid && !process_table::contain_process_group(&pgid) {
        return_errno_with_message!(Errno::EPERM, "process group must exist");
    }

    let process = process_table::get_process(&pid)
        .ok_or(Error::with_message(Errno::ESRCH, "process does not exist"))?;

    process.to_other_group(pgid)?;

    Ok(SyscallReturn::Return(0))
}
