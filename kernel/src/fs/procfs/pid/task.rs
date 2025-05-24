// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use super::*;
use crate::{
    fs::{
        procfs::template::{DirOps, ProcDir, ProcDirBuilder},
        utils::{DirEntryVecExt, Inode},
    },
    process::posix_thread::AsPosixThread,
    Process,
};

/// Represents the inode at `/proc/[pid]/task`.
pub struct TaskDirOps {
    process: Arc<Process>,
    pid_ns: Arc<PidNamespace>,
}

impl TaskDirOps {
    pub fn new_inode(
        process_ref: Arc<Process>,
        pid_ns: Arc<PidNamespace>,
        parent: Weak<dyn Inode>,
    ) -> Arc<dyn Inode> {
        ProcDirBuilder::new(Self {
            process: process_ref,
            pid_ns,
        })
        .parent(parent)
        .build()
        .unwrap()
    }
}

/// Represents the inode at `/proc/[pid]/task/[tid]`.
struct ThreadDirOps(Arc<Process>);

impl ThreadDirOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcDirBuilder::new(Self(process_ref))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl DirOps for ThreadDirOps {
    fn lookup_child(&self, this_ptr: Weak<dyn Inode>, name: &str) -> Result<Arc<dyn Inode>> {
        let inode = match name {
            "fd" => FdDirOps::new_inode(self.0.clone(), this_ptr.clone()),
            "exe" => ExeSymOps::new_inode(self.0.clone(), this_ptr.clone()),
            _ => return_errno!(Errno::ENOENT),
        };
        Ok(inode)
    }

    fn populate_children(&self, this_ptr: Weak<dyn Inode>) {
        let this = {
            let this = this_ptr.upgrade().unwrap();
            this.downcast_ref::<ProcDir<ThreadDirOps>>().unwrap().this()
        };
        let mut cached_children = this.cached_children().write();
        cached_children.put_entry_if_not_found("fd", || {
            FdDirOps::new_inode(self.0.clone(), this_ptr.clone())
        });
        cached_children.put_entry_if_not_found("exe", || {
            ExeSymOps::new_inode(self.0.clone(), this_ptr.clone())
        });
    }
}

impl DirOps for TaskDirOps {
    fn lookup_child(&self, this_ptr: Weak<dyn Inode>, name: &str) -> Result<Arc<dyn Inode>> {
        let Ok(tid) = name.parse::<u32>() else {
            return_errno_with_message!(Errno::ENOENT, "Can not parse name to u32 type");
        };

        for task in self.process.tasks().lock().as_slice() {
            if task
                .as_posix_thread()
                .unwrap()
                .tid_in_ns(&self.pid_ns)
                .unwrap()
                != tid
            {
                continue;
            }
            return Ok(ThreadDirOps::new_inode(self.process.clone(), this_ptr));
        }
        return_errno_with_message!(Errno::ENOENT, "No such thread")
    }

    fn populate_children(&self, this_ptr: Weak<dyn Inode>) {
        let this = {
            let this = this_ptr.upgrade().unwrap();
            this.downcast_ref::<ProcDir<TaskDirOps>>().unwrap().this()
        };
        let mut cached_children = this.cached_children().write();
        for task in self.process.tasks().lock().as_slice() {
            cached_children.put_entry_if_not_found(
                &format!(
                    "{}",
                    task.as_posix_thread()
                        .unwrap()
                        .tid_in_ns(&self.pid_ns)
                        .unwrap()
                ),
                || ThreadDirOps::new_inode(self.process.clone(), this_ptr.clone()),
            );
        }
    }
}
