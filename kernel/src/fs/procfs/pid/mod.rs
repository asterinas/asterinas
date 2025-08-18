// SPDX-License-Identifier: MPL-2.0

use super::template::{DirOps, ProcDir, ProcDirBuilder};
use crate::{
    events::Observer,
    fs::{
        file_table::FdEvents,
        procfs::pid::{
            stat::StatFileOps,
            task::{TaskDirOps, TidDirOps},
        },
        utils::{DirEntryVecExt, Inode},
    },
    prelude::*,
    process::{posix_thread::AsPosixThread, Process},
};

mod stat;
mod task;

/// Represents the inode at `/proc/[pid]`.
pub struct PidDirOps(
    // The `/proc/<pid>` directory is a superset of the `/proc/<pid>/task/<tid>` directory.
    // So we embed `TidDirOps` here so that `PidDirOps` can "inherit" entries and methods
    // from `TidDirOps`.
    TidDirOps,
);

impl PidDirOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        let tid_dir_ops = {
            let thread_ref = process_ref.main_thread();
            TidDirOps {
                process_ref,
                thread_ref,
            }
        };
        let file_table = tid_dir_ops
            .thread_ref
            .as_posix_thread()
            .unwrap()
            .file_table();

        let pid_inode = ProcDirBuilder::new(Self(tid_dir_ops.clone()))
            .parent(parent)
            // The pid directories must be volatile, because it is just associated with one process.
            .volatile()
            .build()
            .unwrap();
        // This is for an exiting process that has not yet been reaped by its parent,
        // whose file table may have already been released.
        if let Some(file_table_ref) = file_table.lock().as_ref() {
            file_table_ref
                .read()
                .register_observer(Arc::downgrade(&pid_inode) as _);
        }

        pid_inode
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

impl DirOps for PidDirOps {
    fn lookup_child(&self, this_ptr: Weak<dyn Inode>, name: &str) -> Result<Arc<dyn Inode>> {
        // Look up entries that either exist under `/proc/<pid>`
        // but not under `/proc/<pid>/task/<tid>`,
        // or entries whose contents differ between `/proc/<pid>` and `/proc/<pid>/task/<tid>`.
        match name {
            "stat" => {
                return Ok(StatFileOps::new_inode(
                    self.0.process_ref.clone(),
                    self.0.thread_ref.clone(),
                    true,
                    this_ptr,
                ))
            }
            "task" => return Ok(TaskDirOps::new_inode(self.0.process_ref.clone(), this_ptr)),
            _ => {}
        }

        // For all other children, the content is the same under both `/proc/<pid>` and `/proc/<pid>/task/<tid>`.
        self.0.lookup_child(this_ptr, name)
    }

    fn populate_children(&self, this_ptr: Weak<dyn Inode>) {
        let this = {
            let this = this_ptr.upgrade().unwrap();
            this.downcast_ref::<ProcDir<PidDirOps>>().unwrap().this()
        };
        let mut cached_children = this.cached_children().write();

        // Populate entries that either exist under `/proc/<pid>`
        // but not under `/proc/<pid>/task/<tid>`,
        // or whose contents differ between the two paths.
        cached_children.put_entry_if_not_found("stat", || {
            StatFileOps::new_inode(
                self.0.process_ref.clone(),
                self.0.thread_ref.clone(),
                true,
                this_ptr.clone(),
            )
        });
        cached_children.put_entry_if_not_found("task", || {
            TaskDirOps::new_inode(self.0.process_ref.clone(), this_ptr.clone())
        });

        // Populate the remaining children that are identical
        // under both `/proc/<pid>` and `/proc/<pid>/task/<tid>`.
        self.0
            .populate_children_inner(&mut cached_children, this_ptr);
    }
}
