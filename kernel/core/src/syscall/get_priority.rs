// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{Pgid, Pid, Process, Uid, pid_table, posix_thread::AsPosixThread},
    sched::Nice,
};

pub(super) fn sys_get_priority(which: i32, who: u32, ctx: &Context) -> Result<SyscallReturn> {
    let prio_target = PriorityTarget::new(which, who, ctx)?;
    debug!("get_priority: prio_target = {:?}", prio_target);

    let processes = get_processes(prio_target)?;
    let highest_prio = {
        let mut nice = Nice::MAX.value().get();
        for process in processes.iter() {
            let proc_nice = process.nice().load(Ordering::Relaxed).value().get();
            // Returns the highest priority (i.e., the lowest numerical value)
            // enjoyed by the processes.
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
    let processes = collect_processes(prio_target);
    if processes.is_empty() {
        return_errno_with_message!(Errno::ESRCH, "the target process does not exist");
    }
    Ok(processes)
}

fn collect_processes(prio_target: PriorityTarget) -> Vec<Arc<Process>> {
    match prio_target {
        PriorityTarget::Process(pid) => {
            let Some(process) = pid_table::pid_table_mut().get_process(pid) else {
                return Vec::new();
            };
            vec![process]
        }
        PriorityTarget::ProcessGroup(pgid) => {
            let Some(process_group) = pid_table::pid_table_mut().get_process_group(&pgid) else {
                return Vec::new();
            };
            process_group.lock().iter().collect()
        }
        PriorityTarget::User(uid) => {
            // Lock order: PID table -> tasks of process
            pid_table::pid_table_mut()
                .iter_processes()
                .filter(|process| {
                    let main_thread = process.main_thread();
                    let posix_thread = main_thread.as_posix_thread().unwrap();
                    uid == posix_thread.credentials().ruid()
                })
                .collect()
        }
    }
}

#[derive(Debug)]
pub(super) enum PriorityTarget {
    /// Apply to the process of the specified process ID.
    Process(Pid),
    /// Apply to the processes that belong to the specified process group.
    ProcessGroup(Pgid),
    /// Apply to the processes that are running under the specified user.
    User(Uid),
}

impl PriorityTarget {
    pub(super) fn new(which: i32, who: u32, ctx: &Context) -> Result<Self> {
        let which = Which::try_from(which)
            .map_err(|_| Error::with_message(Errno::EINVAL, "invalid which value"))?;
        match which {
            Which::PRIO_PROCESS => {
                let pid = if who == 0 {
                    ctx.process.pid()
                } else {
                    who as Pid
                };
                Ok(Self::Process(pid))
            }
            Which::PRIO_PGRP => {
                let pgid = if who == 0 {
                    ctx.process.pgid()
                } else {
                    who as Pgid
                };
                Ok(Self::ProcessGroup(pgid))
            }
            Which::PRIO_USER => {
                let uid = if who == 0 {
                    ctx.posix_thread.credentials().ruid()
                } else {
                    Uid::new(who)
                };
                Ok(Self::User(uid))
            }
        }
    }
}

#[expect(non_camel_case_types)]
#[repr(i32)]
#[derive(Clone, Debug, TryFromInt)]
enum Which {
    PRIO_PROCESS = 0,
    PRIO_PGRP = 1,
    PRIO_USER = 2,
}
