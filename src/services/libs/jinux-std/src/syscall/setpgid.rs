use crate::{
    prelude::*,
    process::{process_group::ProcessGroup, table, Pgid, Pid},
};

use super::{SyscallReturn, SYS_SETPGID};

pub fn sys_setpgid(pid: Pid, pgid: Pgid) -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_SETPGID]", SYS_SETPGID);
    let current = current!();
    // if pid is 0, pid should be the pid of current process
    let pid = if pid == 0 { current.pid() } else { pid };
    // if pgid is 0, pgid should be pid
    let pgid = if pgid == 0 { pid } else { pgid };
    debug!("pid = {}, pgid = {}", pid, pgid);

    if current.pid() != pid {
        return_errno_with_message!(
            Errno::EACCES,
            "cannot set pgid for process other than current"
        );
    }

    // only can move process to an existing group or self
    if pgid != pid && table::pgid_to_process_group(pgid).is_none() {
        return_errno_with_message!(Errno::EPERM, "process group must exist");
    }

    if let Some(new_process_group) = table::pgid_to_process_group(pgid) {
        new_process_group.add_process(current.clone());
        current.set_process_group(Arc::downgrade(&new_process_group));
    } else {
        let new_process_group = Arc::new(ProcessGroup::new(current.clone()));
        new_process_group.add_process(current.clone());
        current.set_process_group(Arc::downgrade(&new_process_group));
        table::add_process_group(new_process_group);
    }

    Ok(SyscallReturn::Return(0))
}
