// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        file::{InodeType, mkmod},
        procfs::{
            ProcDir, StaticEntry,
            sys::vm::{
                max_map_count::MaxMapCountFileOps, mmap_min_addr::MmapMinAddrFileOps,
                overcommit_memory::OvercommitMemoryFileOps,
            },
            template::{
                ProcDirOps, ReaddirEntry, listed_entries_from_table, lookup_child_from_table,
                visit_listed_entries,
            },
        },
        vfs::inode::Inode,
    },
    prelude::*,
};

mod max_map_count;
mod mmap_min_addr;
mod overcommit_memory;

/// Represents the inode at `/proc/sys/vm`.
pub struct VmDirOps;

impl VmDirOps {
    pub fn new_inode(parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        // Reference:
        // <https://elixir.bootlin.com/linux/v6.16.5/source/security/min_addr.c#L59>
        // <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/proc_sysctl.c#L978>
        ProcDir::new(Self, parent, mkmod!(a+rx))
    }

    const STATIC_ENTRIES: &'static [StaticEntry] = &[
        (
            "max_map_count",
            InodeType::File,
            MaxMapCountFileOps::new_inode,
        ),
        (
            "mmap_min_addr",
            InodeType::File,
            MmapMinAddrFileOps::new_inode,
        ),
        (
            "overcommit_memory",
            InodeType::File,
            OvercommitMemoryFileOps::new_inode,
        ),
    ];
}

impl ProcDirOps for VmDirOps {
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
