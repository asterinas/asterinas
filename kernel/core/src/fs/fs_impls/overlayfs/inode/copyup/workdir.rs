// SPDX-License-Identifier: MPL-2.0

//! The workdir temporary lifecycle.
//!
//! [`WorkdirTemp`] preserves the created name of one staged object together
//! with its dentry. [`UpperWorkdirInuse::create_workdir_temp`] and
//! [`UpperWorkdirInuse::create_workdir_link_temp`] each generate one
//! counter-derived name and perform one explicit creation, leaving publication
//! to [`UpperWorkdirInuse::publish_workdir_temp`] and removal to
//! [`UpperWorkdirInuse::cleanup_workdir_temp`].
//!
//! The generated name keeps the target-name copy, one `#` separator, and the
//! counter within `NAME_MAX`. Cleanup sits on the caller side of a publish: the
//! recipe that created a temp removes it on its failure path, by the name it
//! recorded and the type read off its inode.
//!
//! Invariants: the workspace is pinned at mount time and lives outside every
//! layer root; workdir temps are never visible entries of any layer.
//!
//! ## References
//!
//! - Linux `ofs->workdir` dentry-ref parity:
//!   <https://elixir.bootlin.com/linux/latest/source/fs/overlayfs/super.c#L663-L803>

use alloc::format;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::{
    fs::{
        file::InodeType,
        fs_impls::overlayfs::{fs::mount::inuse::UpperWorkdirInuse, inode::CreateOp},
        utils::NAME_MAX,
        vfs::{
            inode::{Inode, RenameMode},
            path::Dentry,
        },
    },
    prelude::*,
};

pub(in super::super) struct WorkdirTemp {
    name: String,
    dentry: Arc<Dentry>,
}

pub(in crate::fs::fs_impls::overlayfs) fn workdir_temp_name(
    target_name: &str,
    next_temp: &AtomicU64,
) -> String {
    /// The counter's width in lowercase hex digits: the width of `u64`.
    const TEMP_NAME_COUNTER_HEX_DIGITS: usize = (u64::BITS / 4) as usize;
    /// The bytes the one `#` separator occupies in the generated name.
    const TEMP_NAME_SEPARATOR_BYTES: usize = 1;
    /// The largest byte count the target-name copy may keep within `NAME_MAX`.
    const TEMP_NAME_TARGET_MAX_BYTES: usize =
        NAME_MAX - TEMP_NAME_SEPARATOR_BYTES - TEMP_NAME_COUNTER_HEX_DIGITS;
    let suffix = next_temp.fetch_add(1, Ordering::Relaxed);
    let target_component =
        &target_name[..target_name.floor_char_boundary(TEMP_NAME_TARGET_MAX_BYTES)];
    format!(
        "{target_component}#{suffix:0width$x}",
        width = TEMP_NAME_COUNTER_HEX_DIGITS
    )
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
}

impl UpperWorkdirInuse {
    /// Creates one staged temp under a counter-derived name with the real call `op` selects.
    pub(in super::super) fn create_workdir_temp(
        &self,
        target_name: &str,
        op: CreateOp<'_>,
    ) -> Result<WorkdirTemp> {
        let workspace = self.workdir_workspace()?;
        let name = workdir_temp_name(target_name, self.next_temp_counter());
        let dentry = op.create_child_in(workspace, &name)?;
        Ok(WorkdirTemp { name, dentry })
    }

    /// Creates one staged temp that is a hard link to `source`.
    pub(in super::super) fn create_workdir_link_temp(
        &self,
        target_name: &str,
        source: &Arc<Dentry>,
    ) -> Result<WorkdirTemp> {
        let workspace = self.workdir_workspace()?;
        let name = workdir_temp_name(target_name, self.next_temp_counter());
        let dir = workspace.as_dir_dentry_or_err()?;
        dir.link(source, &name)?;
        let dentry = dir.lookup_child(&name)?;
        Ok(WorkdirTemp { name, dentry })
    }

    /// Publishes one staged temp into `upper_parent` under `name`.
    pub(in super::super) fn publish_workdir_temp(
        &self,
        temp: &WorkdirTemp,
        upper_parent: &Arc<Dentry>,
        name: &str,
        mode: RenameMode,
    ) -> Result<Arc<Dentry>> {
        let workspace = self.workdir_workspace()?;
        workspace.as_dir_dentry_or_err()?.rename(
            temp.name(),
            &upper_parent.as_dir_dentry_or_err()?,
            name,
            mode,
        )?;
        Ok(temp.dentry().clone())
    }

    /// Removes one staged temp from the workspace, choosing `rmdir` or `unlink` by `kind`.
    pub(in super::super) fn cleanup_workdir_temp(
        &self,
        temp_name: &str,
        kind: InodeType,
    ) -> Result<()> {
        let workspace = self.workdir_workspace()?;
        let dir = workspace.as_dir_dentry_or_err()?;
        if kind.is_directory() {
            dir.rmdir(temp_name)
        } else {
            dir.unlink(temp_name)
        }
    }
}
