// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        file::{InodeType, mkmod},
        procfs::{
            ProcDir,
            sys::kernel::{
                cap_last_cap::CapLastCapFileOps, pid_max::PidMaxFileOps, yama::YamaDirOps,
            },
            template::{
                DirOps, ReaddirEntry, StaticDirEntry, listed_entries_from_table,
                lookup_child_from_table, visit_listed_entries,
            },
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
        ProcDir::new(Self, parent, mkmod!(a+rx))
    }

    #[expect(clippy::type_complexity)]
    const STATIC_ENTRIES: &'static [StaticDirEntry<fn(Weak<dyn Inode>) -> Arc<dyn Inode>>] = &[
        (
            "cap_last_cap",
            InodeType::File,
            CapLastCapFileOps::new_inode,
        ),
        ("pid_max", InodeType::File, PidMaxFileOps::new_inode),
        ("yama", InodeType::Dir, YamaDirOps::new_inode),
    ];
}

impl DirOps for KernelDirOps {
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
