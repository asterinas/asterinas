// SPDX-License-Identifier: MPL-2.0

use super::{Pgid, Pid};
use crate::{fs::file_table::get_file_fast, prelude::*, process::PidFile};

#[derive(Debug, Clone)]
pub enum ProcessFilter {
    Any,
    WithPid(Pid),
    WithPgid(Pgid),
    WithPidfd(Arc<PidFile>),
}

impl ProcessFilter {
    // For `waitpid`.
    pub fn from_which_and_id(which: u64, id: u32, ctx: &Context) -> Result<Self> {
        // Reference:
        // <https://elixir.bootlin.com/linux/v6.14.4/source/include/uapi/linux/wait.h#L16-L20>
        const P_ALL: u64 = 0;
        const P_PID: u64 = 1;
        const P_PGID: u64 = 2;
        const P_PIDFD: u64 = 3;

        match which {
            P_ALL => Ok(ProcessFilter::Any),
            P_PID => Ok(ProcessFilter::WithPid(id)),
            P_PGID => Ok(ProcessFilter::WithPgid(id)),
            P_PIDFD => {
                let file = {
                    let mut file_table = ctx.thread_local.borrow_file_table_mut();
                    get_file_fast!(&mut file_table, id.cast_signed()).into_owned()
                };
                let pid_file = Arc::downcast(file).map_err(|_| {
                    Error::with_message(Errno::EINVAL, "the file is not a PID file")
                })?;
                Ok(ProcessFilter::WithPidfd(pid_file))
            }
            _ => return_errno_with_message!(Errno::EINVAL, "the process filter is invalid"),
        }
    }

    // For `wait4` and `kill`.
    pub fn from_id(wait_pid: i32) -> Self {
        // Reference:
        // <https://man7.org/linux/man-pages/man2/waitpid.2.html>
        // <https://man7.org/linux/man-pages/man2/kill.2.html>
        if wait_pid < -1 {
            // "wait for any child process whose process group ID is equal to the absolute value of
            // `pid`"
            ProcessFilter::WithPgid((-wait_pid).cast_unsigned())
        } else if wait_pid == -1 {
            // "wait for any child process"
            ProcessFilter::Any
        } else if wait_pid == 0 {
            // "wait for any child process whose process group ID is equal to that of the calling
            // process at the time of the call to `waitpid()`"
            let pgid = current!().pgid();
            ProcessFilter::WithPgid(pgid)
        } else {
            // "wait for the child whose process ID is equal to the value of `pid`"
            ProcessFilter::WithPid(wait_pid.cast_unsigned())
        }
    }
}
