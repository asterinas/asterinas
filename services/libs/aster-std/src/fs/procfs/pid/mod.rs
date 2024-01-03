// SPDX-License-Identifier: MPL-2.0

use crate::events::Observer;
use crate::fs::file_table::FdEvents;
use crate::fs::utils::{DirEntryVecExt, Inode};
use crate::prelude::*;
use crate::process::Process;

use self::comm::CommFileOps;
use self::exe::ExeSymOps;
use self::fd::FdDirOps;
use super::template::{
    DirOps, FileOps, ProcDir, ProcDirBuilder, ProcFileBuilder, ProcSymBuilder, SymOps,
};

mod comm;
mod exe;
mod fd;

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
        let file_table = process_ref.file_table().lock();
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
        })
    }
}
