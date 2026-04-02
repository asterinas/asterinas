// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        file::mkmod,
        procfs::{
            ProcDir,
            sys::kernel::{
                cap_last_cap::CapLastCapFileOps, pid_max::PidMaxFileOps, yama::YamaDirOps,
            },
            template::{DirOps, ProcDirBuilder, child_names_from_table, lookup_child_from_table},
        },
        vfs::inode::Inode,
    },
    prelude::*,
};

mod cap_last_cap;
mod pid_max;
mod yama;

/// Represents the inode at `/proc/sys/kernel`.
pub struct KernelDirOps;

impl KernelDirOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference:
        // <https://elixir.bootlin.com/linux/v6.16.5/source/kernel/sysctl.c#L1765>
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/proc_sysctl.c#L978>
        ProcDirBuilder::new(Self, mkmod!(a+rx))
            .parent(parent)
            .build()
            .unwrap()
    }

    #[expect(clippy::type_complexity)]
    const STATIC_ENTRIES: &'static [(&'static str, fn(Weak<dyn Inode>) -> Arc<dyn Inode>)] = &[
        ("cap_last_cap", CapLastCapFileOps::new_inode),
        ("pid_max", PidMaxFileOps::new_inode),
        ("yama", YamaDirOps::new_inode),
    ];
}

impl DirOps for KernelDirOps {
    fn lookup_child(&self, dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        if let Some(child) =
            lookup_child_from_table(name, Self::STATIC_ENTRIES, |f| (f)(dir.this_weak().clone()))
        {
            return Ok(child);
        }

        return_errno_with_message!(Errno::ENOENT, "the file does not exist");
    }

    fn child_names(&self, _dir: &ProcDir<Self>) -> Vec<String> {
        child_names_from_table(Self::STATIC_ENTRIES)
    }
}
