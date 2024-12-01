// SPDX-License-Identifier: MPL-2.0

use self::{cmdline::CmdlineFileOps, comm::CommFileOps, exe::ExeSymOps, fd::FdDirOps};
use super::template::{DirOps, ProcDir, ProcDirBuilder};
use crate::{
    events::Observer,
    fs::{
        file_table::FdEvents,
        utils::{DirEntryVecExt, Inode},
    },
    prelude::*,
    process::{posix_thread::AsPosixThread, Process},
};

mod cmdline;
mod comm;
mod exe;
mod fd;
mod stat;
mod status;

/// Represents the inode at `/proc/[pid]`.
pub struct PidDirOps(Arc<Process>);

impl PidDirOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        let pid_inode = ProcDirBuilder::new(Self(process_ref.clone()))
            .parent(parent)
            // The pid directories must be volatile, because it is just associated with one process.
            .volatile()
            .build()
            .unwrap();
        let main_thread = process_ref.main_thread().unwrap();
        let file_table = main_thread.as_posix_thread().unwrap().file_table().lock();
        let weak_ptr = Arc::downgrade(&pid_inode);
        file_table.register_observer(weak_ptr);
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
        let inode = match name {
            "exe" => ExeSymOps::new_inode(self.0.clone(), this_ptr.clone()),
            "comm" => CommFileOps::new_inode(self.0.clone(), this_ptr.clone()),
            "fd" => FdDirOps::new_inode(self.0.clone(), this_ptr.clone()),
            "cmdline" => CmdlineFileOps::new_inode(self.0.clone(), this_ptr.clone()),
            "status" => status::StatusFileOps::new_inode(self.0.clone(), this_ptr.clone()),
            "stat" => stat::StatFileOps::new_inode(self.0.clone(), this_ptr.clone()),
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
            ExeSymOps::new_inode(self.0.clone(), this_ptr.clone())
        });
        cached_children.put_entry_if_not_found("comm", || {
            CommFileOps::new_inode(self.0.clone(), this_ptr.clone())
        });
        cached_children.put_entry_if_not_found("fd", || {
            FdDirOps::new_inode(self.0.clone(), this_ptr.clone())
        });
        cached_children.put_entry_if_not_found("cmdline", || {
            CmdlineFileOps::new_inode(self.0.clone(), this_ptr.clone())
        });
        cached_children.put_entry_if_not_found("status", || {
            status::StatusFileOps::new_inode(self.0.clone(), this_ptr.clone())
        });
        cached_children.put_entry_if_not_found("stat", || {
            stat::StatFileOps::new_inode(self.0.clone(), this_ptr.clone())
        });
    }
}
