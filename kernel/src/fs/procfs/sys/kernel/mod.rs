// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        procfs::{
            sys::kernel::{cap_last_cap::CapLastCapFileOps, pid_max::PidMaxFileOps},
            template::{DirOps, ProcDirBuilder},
            ProcDir,
        },
        utils::{DirEntryVecExt, Inode, InodeMode},
    },
    prelude::*,
};

mod cap_last_cap;
mod pid_max;

/// Represents the inode at `/proc/sys/kernel`.
pub struct KernelDirOps;

impl KernelDirOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference:
        // <https://elixir.bootlin.com/linux/v6.16.5/source/kernel/sysctl.c#L1765>
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/proc_sysctl.c#L978>
        ProcDirBuilder::new(Self, InodeMode::from_bits_truncate(0o555))
            .parent(parent)
            .build()
            .unwrap()
    }
}

impl DirOps for KernelDirOps {
    fn lookup_child(&self, this_ptr: Weak<dyn Inode>, name: &str) -> Result<Arc<dyn Inode>> {
        let inode = match name {
            "cap_last_cap" => CapLastCapFileOps::new_inode(this_ptr.clone()),
            "pid_max" => PidMaxFileOps::new_inode(this_ptr.clone()),
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
        cached_children
            .put_entry_if_not_found("pid_max", || PidMaxFileOps::new_inode(this_ptr.clone()));
    }
}
