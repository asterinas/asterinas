// SPDX-License-Identifier: MPL-2.0

use super::template::{
    DirOps, ProcDir, ProcDirBuilder, lookup_child_from_table, populate_children_from_table,
};
use crate::{
    fs::{
        file::mkmod,
        procfs::pid::task::{TaskDirOps, TidDirOps},
        vfs::inode::Inode,
    },
    prelude::*,
    process::Process,
};

mod task;

/// Represents the inode at `/proc/[pid]`.
pub struct PidDirOps(
    // The `/proc/<pid>` directory is a superset of the `/proc/<pid>/task/<tid>` directory.
    // So we embed `TidDirOps` here so that `PidDirOps` can "inherit" entries and methods
    // from `TidDirOps`.
    TidDirOps,
);

impl PidDirOps {
    pub fn new_inode(process_ref: Arc<Process>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        let tid_dir_ops = TidDirOps {
            process_ref,
            thread_ref: None,
        };
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3493>
        ProcDirBuilder::new(Self(tid_dir_ops.clone()), mkmod!(a+rx))
            .parent(parent)
            // The PID directories are tied to one process and can disappear at any time.
            .need_revalidation()
            .build()
            .unwrap()
    }

    pub(super) fn process_ref(&self) -> &Arc<Process> {
        &self.0.process_ref
    }

    #[expect(clippy::type_complexity)]
    const STATIC_ENTRIES: &'static [(
        &'static str,
        fn(&PidDirOps, Weak<dyn Inode>) -> Arc<dyn Inode>,
    )] = &[("task", TaskDirOps::new_inode)];
}

impl DirOps for PidDirOps {
    fn lookup_child(&self, dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        // Look up entries that either exist under `/proc/<pid>`
        // but not under `/proc/<pid>/task/<tid>`,
        // or entries whose contents differ between `/proc/<pid>` and `/proc/<pid>/task/<tid>`.
        if let Some(child) = lookup_child_from_table(name, Self::STATIC_ENTRIES, |f| {
            (f)(self, dir.this_weak().clone())
        }) {
            return Ok(child);
        }

        // For all other children, the content is the same under both `/proc/<pid>` and `/proc/<pid>/task/<tid>`.
        self.0.lookup_child(dir.this_weak().clone(), name)
    }

    fn populate_children(&self, dir: &ProcDir<Self>) -> Vec<(String, Arc<dyn Inode>)> {
        let mut children = Vec::new();

        // Populate entries that either exist under `/proc/<pid>`
        // but not under `/proc/<pid>/task/<tid>`,
        // or whose contents differ between the two paths.
        populate_children_from_table(&mut children, Self::STATIC_ENTRIES, |f| {
            (f)(self, dir.this_weak().clone())
        });

        // Populate the remaining children that are identical
        // under both `/proc/<pid>` and `/proc/<pid>/task/<tid>`.
        children.extend(self.0.populate_children(dir.this_weak().clone()));

        children
    }
}
