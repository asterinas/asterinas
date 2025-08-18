// SPDX-License-Identifier: MPL-2.0

use super::template::{DirOps, ProcDir, ProcDirBuilder};
use crate::{
    events::Observer,
    fs::{
        file_table::FdEvents,
        procfs::pid::util::{lookup_child_common, populate_children_common, PidOrTid},
        utils::{DirEntryVecExt, Inode},
    },
    prelude::*,
    process::Process,
};

mod cmdline;
mod comm;
mod exe;
mod fd;
mod stat;
mod status;
mod task;
mod util;

/// Represents the inode at `/proc/[pid]`.
pub struct PidDirOps(PidOrTid);

impl PidDirOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        let pid_or_tid = PidOrTid::new_pid(process_ref);
        let file_table = pid_or_tid.posix_thread().file_table();

        let pid_inode = ProcDirBuilder::new(Self(pid_or_tid.clone()))
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
        lookup_child_common(&self.0, this_ptr, name)
    }

    fn populate_children(&self, this_ptr: Weak<dyn Inode>) {
        let this = {
            let this = this_ptr.upgrade().unwrap();
            this.downcast_ref::<ProcDir<PidDirOps>>().unwrap().this()
        };
        let mut cached_children = this.cached_children().write();
        populate_children_common(&self.0, this_ptr, &mut cached_children);
    }
}
