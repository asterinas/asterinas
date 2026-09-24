// SPDX-License-Identifier: MPL-2.0

//! The workdir temporary lifecycle.
//!
//! [`WorkdirTemp`] is the guard of one staged object: it holds the name the object was created
//! under, its dentry, an owned handle of the workspace it lives in, and whether an outlet has
//! already consumed it. Two consuming outlets discharge the guard — [`WorkdirTemp::publish`]
//! renames the object into the upper, and [`WorkdirTemp::into_whiteout`] detaches it into a
//! whiteout handle the workspace never cleans up — and [`Drop`](WorkdirTemp) is the backstop for
//! an early `?` that took neither, removing the temp. Because the workspace handle is owned and
//! has no accessor, those outlets are the only paths that move a guarded name.
//!
//! [`UpperWorkdirInuse::create_workdir_temp`] and
//! [`UpperWorkdirInuse::create_workdir_link_temp`] each generate one counter-derived name and
//! perform one explicit creation; the naming itself belongs to the owner
//! ([`UpperWorkdirInuse::next_temp_name`]).
//!
//! Invariants: the workspace is pinned at mount time and lives outside every layer root; workdir
//! temps are never visible entries of any layer.

use crate::{
    fs::{
        fs_impls::overlayfs::{fs::mount::inuse::UpperWorkdirInuse, inode::CreateOp},
        vfs::{
            inode::{Inode, RenameMode},
            path::Dentry,
        },
    },
    prelude::*,
};

/// The guard of one staged object.
///
/// It holds the name the object was created under, its dentry, an owned handle of the workspace it
/// lives in, and whether an outlet has already consumed it.
pub(in super::super) struct WorkdirTemp {
    name: String,
    dentry: Arc<Dentry>,
    workspace: Arc<Dentry>,
    consumed: bool,
}

impl WorkdirTemp {
    pub(in super::super) fn name(&self) -> &str {
        &self.name
    }

    pub(in super::super) fn inode(&self) -> &Arc<dyn Inode> {
        self.dentry.inode()
    }

    /// The temp's own dentry, paired with [`Self::inode`] for real-layer setter calls.
    pub(in super::super) fn dentry(&self) -> &Arc<Dentry> {
        &self.dentry
    }

    /// Marks this guard consumed without touching the workspace entry.
    ///
    /// The outlet that detaches the staged object (a shared whiteout outlives the guard) calls
    /// this, so `Drop` leaves the entry where it is.
    pub(in super::super) fn mark_consumed(&mut self) {
        self.consumed = true;
    }

    /// Publishes this staged temp into `upper_parent` under `name`, consuming the guard.
    ///
    /// A successful rename moves the entry out of the workspace, and a failed one leaves the
    /// guard unconsumed so `Drop` removes the temp — the same best-effort cleanup the callers
    /// used to write by hand.
    pub(in super::super) fn publish(
        mut self,
        upper_parent: &Arc<Dentry>,
        name: &str,
        mode: RenameMode,
    ) -> Result<()> {
        self.workspace.as_dir_dentry_or_err()?.rename(
            &self.name,
            &upper_parent.as_dir_dentry_or_err()?,
            name,
            mode,
        )?;
        self.consumed = true;
        Ok(())
    }
}

impl Drop for WorkdirTemp {
    /// Removes a temp that was neither published nor detached.
    fn drop(&mut self) {
        if self.consumed {
            return;
        }
        self.consumed = true;
        let dir = match self.workspace.as_dir_dentry_or_err() {
            Ok(dir) => dir,
            Err(_) => return,
        };
        let _ = if self.dentry.inode().type_().is_directory() {
            dir.rmdir(&self.name)
        } else {
            dir.unlink(&self.name)
        };
    }
}

impl UpperWorkdirInuse {
    /// Creates one staged temp under a counter-derived name with the real call `op` selects.
    pub(in super::super) fn create_workdir_temp(
        &self,
        target_name: &str,
        op: CreateOp<'_>,
    ) -> Result<WorkdirTemp> {
        let workspace = self.workdir_workspace()?;
        let name = self.next_temp_name(target_name);
        let dentry = op.create_child_in(workspace, &name)?;
        Ok(WorkdirTemp {
            name,
            dentry,
            workspace: Arc::clone(workspace),
            consumed: false,
        })
    }

    /// Creates one staged temp that is a hard link to `source`.
    pub(in super::super) fn create_workdir_link_temp(
        &self,
        target_name: &str,
        source: &Arc<Dentry>,
    ) -> Result<WorkdirTemp> {
        let workspace = self.workdir_workspace()?;
        let name = self.next_temp_name(target_name);
        let dir = workspace.as_dir_dentry_or_err()?;
        dir.link(source, &name)?;
        let dentry = dir.lookup_child(&name)?;
        Ok(WorkdirTemp {
            name,
            dentry,
            workspace: Arc::clone(workspace),
            consumed: false,
        })
    }
}
