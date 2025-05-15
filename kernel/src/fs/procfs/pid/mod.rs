// SPDX-License-Identifier: MPL-2.0

use self::{
    cmdline::CmdlineFileOps, comm::CommFileOps, exe::ExeSymOps, fd::FdDirOps, task::TaskDirOps,
};
use super::{
    template::{DirOps, ProcDir, ProcDirBuilder},
    RootDirOps,
};
use crate::{
    events::Observer,
    fs::{
        file_table::FdEvents,
        utils::{DirEntryVecExt, Inode},
    },
    prelude::*,
    process::{posix_thread::AsPosixThread, Pid, PidEvent, PidNamespace, Process, TASK_LIST_LOCK},
};

mod cmdline;
mod comm;
mod exe;
mod fd;
mod stat;
mod status;
mod task;

/// Represents the inode at `/proc/[pid]`.
pub struct PidDirOps {
    pid: Pid,
    process: Arc<Process>,
    pid_ns: Arc<PidNamespace>,
}

impl PidDirOps {
    pub fn new_inode(
        pid: Pid,
        pid_ns: Arc<PidNamespace>,
        parent: Weak<dyn Inode>,
    ) -> Option<Arc<dyn Inode>> {
        let map = pid_ns.get_entry_by_id(pid)?;
        // FIXME: We should hold the task list lock until `register_observer`
        // instead of releasing it here to avoid race conditions.
        // However, we will acquire the file table lock below, which is a `Mutex` lock.
        // If we don't release the task list lock, it will break the atomic mode.
        let process_ref = map
            .with_task_list_guard(&mut TASK_LIST_LOCK.lock())
            .attached_process()?;

        let main_thread = process_ref.main_thread();
        let file_table = main_thread.as_posix_thread().unwrap().file_table();

        let pid_inode = ProcDirBuilder::new(Self {
            pid,
            process: process_ref.clone(),
            pid_ns,
        })
        .parent(parent)
        // The pid directories must be volatile, because it is just associated with one process.
        .volatile()
        .build()
        .unwrap();
        file_table
            .lock()
            .as_ref()
            .unwrap()
            .read()
            .register_observer(Arc::downgrade(&pid_inode) as _);
        map.register_observer(Arc::downgrade(&pid_inode) as _);

        Some(pid_inode)
    }
}

impl Observer<FdEvents> for ProcDir<PidDirOps> {
    fn on_events(&self, events: &FdEvents) {
        if let FdEvents::DropFileTable = events {
            let mut cached_children = self.cached_children().write();
            cached_children.remove_entry_by_name("fd");
        }
    }
}

impl Observer<PidEvent> for ProcDir<PidDirOps> {
    fn on_events(&self, event: &PidEvent) {
        let PidEvent::Exit = event;

        let Some(root_inode) = self.parent() else {
            return;
        };

        let pid = {
            let pid_dir_ops = self.dir();
            pid_dir_ops.pid
        };

        let root_inode = root_inode.downcast_ref::<ProcDir<RootDirOps>>().unwrap();
        let mut cached_children = root_inode.cached_children().write();
        cached_children.remove_entry_by_name(&pid.to_string());
    }
}

impl DirOps for PidDirOps {
    fn lookup_child(&self, this_ptr: Weak<dyn Inode>, name: &str) -> Result<Arc<dyn Inode>> {
        let inode = match name {
            "exe" => ExeSymOps::new_inode(self.process.clone(), this_ptr.clone()),
            "comm" => CommFileOps::new_inode(self.process.clone(), this_ptr.clone()),
            "fd" => FdDirOps::new_inode(self.process.clone(), this_ptr.clone()),
            "cmdline" => CmdlineFileOps::new_inode(self.process.clone(), this_ptr.clone()),
            "status" => status::StatusFileOps::new_inode(
                self.process.clone(),
                self.pid_ns.clone(),
                this_ptr.clone(),
            ),
            "stat" => stat::StatFileOps::new_inode(
                self.process.clone(),
                self.pid_ns.clone(),
                this_ptr.clone(),
            ),
            "task" => {
                TaskDirOps::new_inode(self.process.clone(), self.pid_ns.clone(), this_ptr.clone())
            }
            _ => return_errno!(Errno::ENOENT),
        };
        Ok(inode)
    }

    fn populate_children(&self, this_ptr: Weak<dyn Inode>) {
        let this = {
            let this = this_ptr.upgrade().unwrap();
            this.downcast_ref::<ProcDir<PidDirOps>>().unwrap().this()
        };
        let mut cached_children = this.cached_children().write();
        cached_children.put_entry_if_not_found("exe", || {
            ExeSymOps::new_inode(self.process.clone(), this_ptr.clone())
        });
        cached_children.put_entry_if_not_found("comm", || {
            CommFileOps::new_inode(self.process.clone(), this_ptr.clone())
        });
        cached_children.put_entry_if_not_found("fd", || {
            FdDirOps::new_inode(self.process.clone(), this_ptr.clone())
        });
        cached_children.put_entry_if_not_found("cmdline", || {
            CmdlineFileOps::new_inode(self.process.clone(), this_ptr.clone())
        });
        cached_children.put_entry_if_not_found("status", || {
            status::StatusFileOps::new_inode(
                self.process.clone(),
                self.pid_ns.clone(),
                this_ptr.clone(),
            )
        });
        cached_children.put_entry_if_not_found("stat", || {
            stat::StatFileOps::new_inode(
                self.process.clone(),
                self.pid_ns.clone(),
                this_ptr.clone(),
            )
        });
        cached_children.put_entry_if_not_found("task", || {
            TaskDirOps::new_inode(self.process.clone(), self.pid_ns.clone(), this_ptr.clone())
        });
    }
}
