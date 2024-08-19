// SPDX-License-Identifier: MPL-2.0

use self::kernel::KernelDirOps;
use crate::{
    fs::{
        procfs::template::{DirOps, ProcDir, ProcDirBuilder},
        utils::{DirEntryVecExt, Inode},
    },
    prelude::*,
};

mod kernel;

/// Represents the inode at `/proc/sys`.
pub struct SysDirOps;

impl SysDirOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcDirBuilder::new(Self).parent(parent).build().unwrap()
    }
}

impl DirOps for SysDirOps {
    fn lookup_child(&self, this_ptr: Weak<dyn Inode>, name: &str) -> Result<Arc<dyn Inode>> {
        let inode = match name {
            "kernel" => KernelDirOps::new_inode(this_ptr.clone()),
            _ => return_errno!(Errno::ENOENT),
        };
        Ok(inode)
    }

    fn populate_children(&self, this_ptr: Weak<dyn Inode>) {
        let this = {
            let this = this_ptr.upgrade().unwrap();
            this.downcast_ref::<ProcDir<SysDirOps>>().unwrap().this()
        };
        let mut cached_children = this.cached_children().write();
        cached_children
            .put_entry_if_not_found("kernel", || KernelDirOps::new_inode(this_ptr.clone()))
    }
}
