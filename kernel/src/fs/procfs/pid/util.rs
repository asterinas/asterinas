// SPDX-License-Identifier: MPL-2.0

use aster_util::slot_vec::SlotVec;

use crate::{
    fs::{
        procfs::pid::{
            cmdline::CmdlineFileOps, comm::CommFileOps, exe::ExeSymOps, fd::FdDirOps,
            stat::StatFileOps, status::StatusFileOps, task::TaskDirOps,
        },
        utils::{DirEntryVecExt, Inode},
    },
    prelude::*,
    process::{
        posix_thread::{AsPosixThread, PosixThread},
        Process,
    },
    thread::Thread,
};

/// Represents an inode is under `/proc/[pid]` or `/proc/[pid]/task/[tid]`.
#[derive(Clone)]
pub enum PidOrTid {
    Pid {
        process: Arc<Process>,
        main_thread: Arc<Thread>,
    },
    Tid {
        process: Arc<Process>,
        thread: Arc<Thread>,
    },
}

impl PidOrTid {
    pub(super) fn new_pid(process: Arc<Process>) -> Self {
        let main_thread = process.main_thread();
        Self::Pid {
            process,
            main_thread,
        }
    }

    pub(super) fn new_tid(process: Arc<Process>, thread: Arc<Thread>) -> Self {
        debug_assert!(Arc::ptr_eq(
            &process,
            &thread.as_posix_thread().unwrap().process()
        ));

        Self::Tid { process, thread }
    }

    pub(super) fn process(&self) -> &Arc<Process> {
        match self {
            PidOrTid::Pid { process, .. } | PidOrTid::Tid { process, .. } => process,
        }
    }

    pub(super) fn thread(&self) -> &Arc<Thread> {
        match self {
            PidOrTid::Pid {
                main_thread: thread,
                ..
            }
            | PidOrTid::Tid { thread, .. } => thread,
        }
    }

    pub(super) fn posix_thread(&self) -> &PosixThread {
        self.thread().as_posix_thread().unwrap()
    }

    pub(super) fn is_pid(&self) -> bool {
        matches!(self, Self::Pid { .. })
    }
}

pub(super) fn lookup_child_common(
    pid_or_tid: &PidOrTid,
    this_ptr: Weak<dyn Inode>,
    name: &str,
) -> Result<Arc<dyn Inode>> {
    let inode = match name {
        "cmdline" => CmdlineFileOps::new_inode(pid_or_tid.clone(), this_ptr),
        "comm" => CommFileOps::new_inode(pid_or_tid.clone(), this_ptr),
        "exe" => ExeSymOps::new_inode(pid_or_tid.clone(), this_ptr),
        "fd" => FdDirOps::new_inode(pid_or_tid.clone(), this_ptr),
        "stat" => StatFileOps::new_inode(pid_or_tid.clone(), this_ptr),
        "status" => StatusFileOps::new_inode(pid_or_tid.clone(), this_ptr),
        "task" if pid_or_tid.is_pid() => {
            TaskDirOps::new_inode(pid_or_tid.process().clone(), this_ptr)
        }
        _ => return_errno!(Errno::ENOENT),
    };
    Ok(inode)
}

pub(super) fn populate_children_common(
    pid_or_tid: &PidOrTid,
    this_ptr: Weak<dyn Inode>,
    cached_children: &mut SlotVec<(String, Arc<dyn Inode>)>,
) {
    cached_children.put_entry_if_not_found("cmdline", || {
        CmdlineFileOps::new_inode(pid_or_tid.clone(), this_ptr.clone())
    });
    cached_children.put_entry_if_not_found("comm", || {
        CommFileOps::new_inode(pid_or_tid.clone(), this_ptr.clone())
    });
    cached_children.put_entry_if_not_found("exe", || {
        ExeSymOps::new_inode(pid_or_tid.clone(), this_ptr.clone())
    });
    cached_children.put_entry_if_not_found("fd", || {
        FdDirOps::new_inode(pid_or_tid.clone(), this_ptr.clone())
    });
    cached_children.put_entry_if_not_found("stat", || {
        StatFileOps::new_inode(pid_or_tid.clone(), this_ptr.clone())
    });
    cached_children.put_entry_if_not_found("status", || {
        StatusFileOps::new_inode(pid_or_tid.clone(), this_ptr.clone())
    });
    if pid_or_tid.is_pid() {
        cached_children.put_entry_if_not_found("task", || {
            TaskDirOps::new_inode(pid_or_tid.process().clone(), this_ptr.clone())
        });
    }
}
