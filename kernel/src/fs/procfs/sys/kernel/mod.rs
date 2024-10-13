// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        procfs::{
            sys::kernel::cap_last_cap::CapLastCapFileOps,
            template::{DirOps, ProcDirBuilder},
            ProcDir,
        },
        utils::{DirEntryVecExt, Inode},
    },
    prelude::*,
};

mod cap_last_cap;

/// Represents the inode at `/proc/sys/kernel`.
pub struct KernelDirOps;

impl KernelDirOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        ProcDirBuilder::new(Self).parent(parent).build().unwrap()
    }
}

impl DirOps for KernelDirOps {
    fn lookup_child(&self, this_ptr: Weak<dyn Inode>, name: &str) -> Result<Arc<dyn Inode>> {
        let inode = match name {
            "cap_last_cap" => CapLastCapFileOps::new_inode(this_ptr.clone()),
            _ => return_errno!(Errno::ENOENT),
        };
        Ok(inode)
    }

    fn populate_children(&self, this_ptr: Weak<dyn Inode>) {
        let this = {
            let this = this_ptr.upgrade().unwrap();
            this.downcast_ref::<ProcDir<KernelDirOps>>().unwrap().this()
        };
        let mut cached_children = this.cached_children().write();
        cached_children.put_entry_if_not_found("cap_last_cap", || {
            CapLastCapFileOps::new_inode(this_ptr.clone())
        });
    }
}
