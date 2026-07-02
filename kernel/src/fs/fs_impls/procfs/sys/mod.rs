// SPDX-License-Identifier: MPL-2.0

use self::{fs::FsDirOps, kernel::KernelDirOps};
use super::{
    StaticEntry,
    template::{ReaddirEntry, listed_entries_from_table, visit_listed_entries},
};
use crate::{
    fs::{
        file::{InodeType, mkmod},
        procfs::template::{ProcDir, ProcDirOps, lookup_child_from_table},
        vfs::inode::Inode,
    },
    prelude::*,
};

mod fs;
mod kernel;

/// Represents the inode at `/proc/sys`.
pub struct SysDirOps;

impl SysDirOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference:
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/proc_sysctl.c#L1566>
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/generic.c#L488-L489>
        ProcDir::new(Self, parent, mkmod!(a+rx))
    }

    const STATIC_ENTRIES: &'static [StaticEntry] = &[
        ("fs", InodeType::Dir, FsDirOps::new_inode),
        ("kernel", InodeType::Dir, KernelDirOps::new_inode),
    ];
}

impl ProcDirOps for SysDirOps {
    fn lookup_child(&self, this_dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        if let Some(child) = lookup_child_from_table(name, Self::STATIC_ENTRIES, |f| {
            (f)(this_dir.this_weak().clone())
        }) {
            return Ok(child);
        }

        return_errno_with_message!(Errno::ENOENT, "the file does not exist");
    }

    fn visit_entries_from_offset<'a, F>(&'a self, offset: usize, visit_fn: F) -> Result<()>
    where
        F: FnMut(ReaddirEntry<'a>) -> Result<()>,
    {
        visit_listed_entries(
            offset,
            listed_entries_from_table(Self::STATIC_ENTRIES),
            visit_fn,
        )
    }
}
