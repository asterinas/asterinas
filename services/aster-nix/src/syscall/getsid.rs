// SPDX-License-Identifier: MPL-2.0

use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::{process_table, Pid};

use super::{SyscallReturn, SYS_GETSID};

pub fn sys_getsid(pid: Pid) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GETSID);
    debug!("pid = {}", pid);

    let session = current!().session().unwrap();
    let sid = session.sid();

    if pid == 0 {
        return Ok(SyscallReturn::Return(sid as _));
    }

    let Some(process) = process_table::get_process(&pid) else {
        return_errno_with_message!(Errno::ESRCH, "the process does not exist")
    };

    if !Arc::ptr_eq(&session, &process.session().unwrap()) {
        return_errno_with_message!(
            Errno::EPERM,
            "the process and current process does not belong to the same session"
        );
    }

    Ok(SyscallReturn::Return(sid as _))
}
