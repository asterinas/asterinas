use crate::{
    log_syscall_entry,
    prelude::*,
    process::{process_table, Pgid, Pid, ProcessGroup},
};

use super::{SyscallReturn, SYS_SETPGID};

pub fn sys_setpgid(pid: Pid, pgid: Pgid) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SETPGID);
    let current = current!();
    // if pid is 0, pid should be the pid of current process
    let pid = if pid == 0 { current.pid() } else { pid };
    // if pgid is 0, pgid should be pid
    let pgid = if pgid == 0 { pid } else { pgid };
    debug!("pid = {}, pgid = {}", pid, pgid);

    if pid != current.pid() && !current.children().lock().contains_key(&pid) {
        return_errno_with_message!(
            Errno::ESRCH,
            "cannot set pgid for process other than current or children of current"
        );
    }
    // FIXME: If pid is child process of current and already calls execve, should return error.
    // How can we determine a child process has called execve?

    // only can move process to an existing group or self
    if pgid != pid && process_table::pgid_to_process_group(pgid).is_none() {
        return_errno_with_message!(Errno::EPERM, "process group must exist");
    }

    let process = process_table::pid_to_process(pid)
        .ok_or(Error::with_message(Errno::ESRCH, "process does not exist"))?;

    // if the process already belongs to the process group
    if process.pgid() == pgid {
        return Ok(SyscallReturn::Return(0));
    }

    if let Some(process_group) = process_table::pgid_to_process_group(pgid) {
        process_group.add_process(process.clone());
        process.set_process_group(Arc::downgrade(&process_group));
    } else {
        let new_process_group = Arc::new(ProcessGroup::new(process.clone()));
        // new_process_group.add_process(process.clone());
        process.set_process_group(Arc::downgrade(&new_process_group));
        process_table::add_process_group(new_process_group);
    }

    Ok(SyscallReturn::Return(0))
}
