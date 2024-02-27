// SPDX-License-Identifier: MPL-2.0

use super::{Pgid, Pid};
use crate::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessFilter {
    Any,
    WithPid(Pid),
    WithPgid(Pgid),
}

impl ProcessFilter {
    // used for waitid
    pub fn from_which_and_id(which: u64, id: u64) -> Self {
        // Does not support PID_FD now(which = 3)
        // https://elixir.bootlin.com/linux/latest/source/include/uapi/linux/wait.h#L20
        match which {
            0 => ProcessFilter::Any,
            1 => ProcessFilter::WithPid(id as Pid),
            2 => ProcessFilter::WithPgid(id as Pgid),
            _ => panic!("Unknown id type"),
        }
    }

    // used for wait4 and kill
    pub fn from_id(wait_pid: i32) -> Self {
        // https://man7.org/linux/man-pages/man2/waitpid.2.html
        // https://man7.org/linux/man-pages/man2/kill.2.html
        if wait_pid < -1 {
            // process group ID is equal to the absolute value of pid.
            ProcessFilter::WithPgid((-wait_pid) as Pgid)
        } else if wait_pid == -1 {
            // wait for any child process
            ProcessFilter::Any
        } else if wait_pid == 0 {
            // wait for any child process with same process group ID
            let pgid = current!().pgid();
            ProcessFilter::WithPgid(pgid)
        } else {
            // pid > 0. wait for the child whose process ID is equal to the value of pid.
            ProcessFilter::WithPid(wait_pid as Pid)
        }
    }

    pub fn contains_pid(&self, pid: Pid) -> bool {
        match self {
            ProcessFilter::Any => true,
            ProcessFilter::WithPid(filter_pid) => *filter_pid == pid,
            ProcessFilter::WithPgid(_) => todo!(),
        }
    }
}
