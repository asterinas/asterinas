// SPDX-License-Identifier: MPL-2.0

use super::template::{
    DirOps, ProcDir, ReaddirEntry, StaticDirEntry, listed_entries_from_table,
    lookup_child_from_table, visit_listed_entries,
};
use crate::{
    fs::{
        file::{InodeType, mkmod},
        procfs::pid::task::TaskDirOps,
        vfs::inode::{Inode, RevalidationPolicy},
    },
    prelude::*,
    process::pid_table::{PidEntry, PidEntryType},
};

mod task;
pub(super) use task::TidDirOps;

/// Represents the inode at `/proc/[pid]`.
pub struct PidDirOps(
    // The `/proc/<pid>` directory is a superset of the `/proc/<pid>/task/<tid>` directory.
    // So we embed `TidDirOps` here so that `PidDirOps` can "inherit" entries and methods
    // from `TidDirOps`.
    TidDirOps,
);

impl PidDirOps {
    pub fn new_inode(pid_entry: Arc<PidEntry>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        let this = Self(TidDirOps::new(pid_entry));
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3493>
        ProcDir::new(this, parent, mkmod!(a+rx))
    }

    pub(super) fn pid_entry(&self) -> &Arc<PidEntry> {
        self.0.pid_entry()
    }

    pub(super) fn tid_dir_ops(&self) -> &TidDirOps {
        &self.0
    }

    #[expect(clippy::type_complexity)]
    const STATIC_ENTRIES: &[StaticDirEntry<fn(&PidDirOps, Weak<dyn Inode>) -> Arc<dyn Inode>>] = &[
        ("task", InodeType::Dir, TaskDirOps::new_inode),
        (
            "stat",
            InodeType::File,
            task::stat::StatFileOps::new_process_inode,
        ),
    ];
}

impl DirOps for PidDirOps {
    fn lookup_child(&self, this_dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        if self.0.pid_entry().type_().is_none() {
            return_errno_with_message!(Errno::ESRCH, "the process does not exist");
        }

        // Look up entries that either exist under `/proc/<pid>`
        // but not under `/proc/<pid>/task/<tid>`,
        // or entries whose contents differ between `/proc/<pid>` and `/proc/<pid>/task/<tid>`.
        if let Some(child) = lookup_child_from_table(name, Self::STATIC_ENTRIES, |f| {
            (f)(self, this_dir.this_weak().clone())
        }) {
            return Ok(child);
        }

        // For all other children, the content is the same under both `/proc/<pid>` and `/proc/<pid>/task/<tid>`.
        self.0
            .lookup_static_child(this_dir.this_weak().clone(), name)
    }

    fn revalidation_policy(&self) -> RevalidationPolicy {
        RevalidationPolicy::REVALIDATE_EXISTS
    }

    fn revalidate_exists(&self, _name: &str, _child: &dyn Inode) -> bool {
        matches!(self.pid_entry().type_(), Some(PidEntryType::Process))
    }

    fn visit_entries_from_offset<'a, F>(&'a self, offset: usize, visit_fn: F) -> Result<()>
    where
        F: FnMut(ReaddirEntry<'a>) -> Result<()>,
    {
        if self.0.pid_entry().type_().is_none() {
            return_errno_with_message!(Errno::ENOENT, "the process does not exist");
        }

        const PROCESS_OVERRIDE_NAMES: &[&str] = &["stat"];

        // Add the children that are inherited from `/proc/<pid>/task/<tid>` but not overridden
        // by process-specific entries under `/proc/<pid>`.
        let listed_entries = listed_entries_from_table(Self::STATIC_ENTRIES).chain(
            self.0
                .static_listed_entries()
                .filter(|entry| !PROCESS_OVERRIDE_NAMES.contains(&entry.name())),
        );

        visit_listed_entries(offset, listed_entries, visit_fn)
    }
}
