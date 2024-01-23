// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use super::{SyscallReturn, SYS_GET_PRIORITY, SYS_SET_PRIORITY};
use crate::{
    log_syscall_entry,
    prelude::*,
    process::{credentials, posix_thread::PosixThreadExt, process_table, Pgid, Pid, Process, Uid},
    sched::nice::Nice,
};

pub fn sys_set_priority(which: i32, who: u32, prio: i32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SET_PRIORITY);
    let prio_target = PriorityTarget::new(which, who)?;
    let new_nice = {
        let norm_prio = if prio > i8::MAX as i32 {
            i8::MAX
        } else if prio < i8::MIN as i32 {
            i8::MIN
        } else {
            prio as i8
        };
        Nice::new(norm_prio)
    };

    debug!(
        "set_priority prio_target: {:?}, new_nice: {:?}",
        prio_target, new_nice
    );

    let processes = get_processes(prio_target)?;
    for process in processes.iter() {
        process.nice().store(new_nice, Ordering::Relaxed);
    }

    Ok(SyscallReturn::Return(0))
}

pub fn sys_get_priority(which: i32, who: u32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GET_PRIORITY);
    let prio_target = PriorityTarget::new(which, who)?;
    debug!("get_priority prio_target: {:?}", prio_target);

    let processes = get_processes(prio_target)?;
    let highest_prio = {
        let mut nice = Nice::MAX;
        for process in processes.iter() {
            let proc_nice = process.nice().load(Ordering::Relaxed);
            // Returns the highest priority enjoyed by the processes
            if proc_nice < nice {
                nice = proc_nice;
            }
        }

        // The system call returns nice values translated to the range 40 to 1,
        // since a negative return value would be interpreted as an error.
        20 - nice.to_raw()
    };

    Ok(SyscallReturn::Return(highest_prio as _))
}

fn get_processes(prio_target: PriorityTarget) -> Result<Vec<Arc<Process>>> {
    Ok(match prio_target {
        PriorityTarget::Process(pid) => {
            let process = process_table::get_process(&pid).ok_or(Error::new(Errno::ESRCH))?;
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
            let processes: Vec<Arc<Process>> = process_table::process_table()
                .iter()
                .filter(|process| {
                    let Some(main_thread) = process.main_thread() else {
                        return false;
                    };
                    let Some(posix_thread) = main_thread.as_posix_thread() else {
                        return false;
                    };
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
enum PriorityTarget {
    Process(Pid),
    ProcessGroup(Pgid),
    User(Uid),
}

impl PriorityTarget {
    fn new(which: i32, who: u32) -> Result<Self> {
        let which = Which::try_from(which)
            .map_err(|_| Error::with_message(Errno::EINVAL, "invalid which value"))?;
        Ok(match which {
            Which::PRIO_PROCESS => {
                let pid = if who == 0 {
                    current!().pid()
                } else {
                    who as Pid
                };
                Self::Process(pid)
            }
            Which::PRIO_PGRP => {
                let pgid = if who == 0 {
                    current!().pgid()
                } else {
                    who as Pgid
                };
                Self::ProcessGroup(pgid)
            }
            Which::PRIO_USER => {
                let uid = if who == 0 {
                    credentials().ruid()
                } else {
                    Uid::new(who)
                };
                Self::User(uid)
            }
        })
    }
}

#[allow(non_camel_case_types)]
#[derive(Clone, Debug, TryFromInt)]
#[repr(i32)]
enum Which {
    PRIO_PROCESS = 0,
    PRIO_PGRP = 1,
    PRIO_USER = 2,
}
