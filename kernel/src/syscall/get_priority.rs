// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{posix_thread::AsPosixThread, process_table, Pgid, Pid, Process, Uid},
    sched::Nice,
};

pub fn sys_get_priority(which: i32, who: u32, ctx: &Context) -> Result<SyscallReturn> {
    let prio_target = PriorityTarget::new(which, who, ctx)?;
    debug!("get_priority prio_target: {:?}", prio_target);

    let processes = get_processes(prio_target)?;
    let highest_prio = {
        let mut nice = Nice::MAX.value().get();
        for process in processes.iter() {
            let proc_nice = process.nice().load(Ordering::Relaxed).value().get();
            // Returns the highest priority enjoyed by the processes
            if proc_nice < nice {
                nice = proc_nice;
            }
        }

        // The system call returns nice values translated to the range 40 to 1,
        // since a negative return value would be interpreted as an error.
        20 - nice
    };

    Ok(SyscallReturn::Return(highest_prio as _))
}

pub(super) fn get_processes(prio_target: PriorityTarget) -> Result<Vec<Arc<Process>>> {
    Ok(match prio_target {
        PriorityTarget::Process(pid) => {
            let process = process_table::get_process(pid).ok_or(Error::new(Errno::ESRCH))?;
            vec![process]
        }
        PriorityTarget::ProcessGroup(pgid) => {
            let process_group =
                process_table::get_process_group(&pgid).ok_or(Error::new(Errno::ESRCH))?;
            let processes: Vec<Arc<Process>> = process_group.lock().iter().cloned().collect();
            if processes.is_empty() {
                return_errno!(Errno::ESRCH);
            }
            processes
        }
        PriorityTarget::User(uid) => {
            // Get the processes that are running under the specified user
            let processes: Vec<Arc<Process>> = process_table::process_table_mut()
                .iter()
                .filter(|process| {
                    let main_thread = process.main_thread();
                    let posix_thread = main_thread.as_posix_thread().unwrap();
                    uid == posix_thread.credentials().ruid()
                })
                .cloned()
                .collect();
            if processes.is_empty() {
                return_errno!(Errno::ESRCH);
            }
            processes
        }
    })
}

#[derive(Debug)]
pub(super) enum PriorityTarget {
    Process(Pid),
    ProcessGroup(Pgid),
    User(Uid),
}

impl PriorityTarget {
    pub(super) fn new(which: i32, who: u32, ctx: &Context) -> Result<Self> {
        let which = Which::try_from(which)
            .map_err(|_| Error::with_message(Errno::EINVAL, "invalid which value"))?;
        Ok(match which {
            Which::PRIO_PROCESS => {
                let pid = if who == 0 {
                    ctx.process.pid()
                } else {
                    who as Pid
                };
                Self::Process(pid)
            }
            Which::PRIO_PGRP => {
                let pgid = if who == 0 {
                    ctx.process.pgid()
                } else {
                    who as Pgid
                };
                Self::ProcessGroup(pgid)
            }
            Which::PRIO_USER => {
                let uid = if who == 0 {
                    ctx.posix_thread.credentials().ruid()
                } else {
                    Uid::new(who)
                };
                Self::User(uid)
            }
        })
    }
}

#[expect(non_camel_case_types)]
#[derive(Clone, Debug, TryFromInt)]
#[repr(i32)]
pub(super) enum Which {
    PRIO_PROCESS = 0,
    PRIO_PGRP = 1,
    PRIO_USER = 2,
}
