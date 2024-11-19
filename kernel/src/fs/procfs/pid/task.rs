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
pub struct TaskDirOps(Arc<Process>);

impl TaskDirOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcDirBuilder::new(Self(process_ref))
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
    }
}

impl DirOps for TaskDirOps {
    fn lookup_child(&self, this_ptr: Weak<dyn Inode>, name: &str) -> Result<Arc<dyn Inode>> {
        for task in self.0.tasks().lock().iter() {
            if task.as_posix_thread().unwrap().tid() != name.parse::<u32>().unwrap() {
                continue;
            }
            return Ok(ThreadDirOps::new_inode(self.0.clone(), this_ptr));
        }
        return_errno_with_message!(Errno::ENOENT, "No such thread")
    }

    fn populate_children(&self, this_ptr: Weak<dyn Inode>) {
        let this = {
            let this = this_ptr.upgrade().unwrap();
            this.downcast_ref::<ProcDir<TaskDirOps>>().unwrap().this()
        };
        let mut cached_children = this.cached_children().write();
        for task in self.0.tasks().lock().iter() {
            cached_children.put_entry_if_not_found(
                &format!("{}", task.as_posix_thread().unwrap().tid()),
                || ThreadDirOps::new_inode(self.0.clone(), this_ptr.clone()),
            );
        }
    }
}
