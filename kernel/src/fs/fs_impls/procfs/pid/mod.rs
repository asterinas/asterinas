// SPDX-License-Identifier: MPL-2.0

use super::template::{
    DirOps, ProcDir, ProcDirBuilder, child_names_from_table, lookup_child_from_table,
};
use crate::{
    fs::{
        file::mkmod,
        procfs::pid::task::{TaskDirOps, TidDirOps},
        vfs::inode::{Inode, RevalidateResult},
    },
    prelude::*,
    process::pid_table::PidEntry,
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
    pub fn new_inode(pid_entry: Arc<PidEntry>, parent: Weak<dyn Inode>) -> Arc<dyn Inode> {
        let this = Self(TidDirOps::new(pid_entry));
        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/base.c#L3493>
        ProcDirBuilder::new(this, mkmod!(a+rx))
            .parent(parent)
            // The PID directories are tied to one process and can disappear at any time.
            .need_revalidation()
            .need_neg_child_revalidation()
            .build()
            .unwrap()
    }

    pub(super) fn pid_entry(&self) -> &Arc<PidEntry> {
        self.0.pid_entry()
    }

    #[expect(clippy::type_complexity)]
    const STATIC_ENTRIES: &[(&str, fn(&PidDirOps, Weak<dyn Inode>) -> Arc<dyn Inode>)] = &[
        ("task", TaskDirOps::new_inode),
        ("stat", task::stat::StatFileOps::new_process_inode),
    ];
}

impl DirOps for PidDirOps {
    fn lookup_child(&self, dir: &ProcDir<Self>, name: &str) -> Result<Arc<dyn Inode>> {
        if self.0.process().is_none() {
            return_errno_with_message!(Errno::ESRCH, "the process has been reaped");
        }

        // Look up entries that either exist under `/proc/<pid>`
        // but not under `/proc/<pid>/task/<tid>`,
        // or entries whose contents differ between `/proc/<pid>` and `/proc/<pid>/task/<tid>`.
        if let Some(child) = lookup_child_from_table(name, Self::STATIC_ENTRIES, |f| {
            (f)(self, dir.this_weak().clone())
        }) {
            return Ok(child);
        }

        // For all other children, the content is the same under both `/proc/<pid>` and `/proc/<pid>/task/<tid>`.
        self.0.lookup_static_child(dir.this_weak().clone(), name)
    }

    fn revalidate_pos_child(&self, _name: &str, _child: &dyn Inode) -> RevalidateResult {
        if self.0.process().is_none() {
            RevalidateResult::Invalid
        } else {
            RevalidateResult::Valid
        }
    }

    fn child_names(&self, _dir: &ProcDir<Self>) -> Vec<String> {
        let mut children = child_names_from_table(Self::STATIC_ENTRIES);

        const PROCESS_OVERRIDE_NAMES: &[&str] = &["stat"];

        // Add the children that are inherited from `/proc/<pid>/task/<tid>` but not overridden
        // by process-specific entries under `/proc/<pid>`.
        children.extend(
            self.0
                .static_child_names()
                .into_iter()
                .filter(|name| !PROCESS_OVERRIDE_NAMES.contains(&name.as_str())),
        );

        children
    }
}
