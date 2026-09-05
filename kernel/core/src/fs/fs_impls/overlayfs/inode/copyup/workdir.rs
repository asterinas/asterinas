// SPDX-License-Identifier: MPL-2.0

//! The workdir temporary lifecycle.
//!
//! [`WorkdirTemp`] preserves the created name, its dentry, and the
//! request-derived kind. [`WorkdirTemp::create_from_op`],
//! [`WorkdirTemp::create_from_link`], and [`WorkdirTemp::create_from_char_device`]
//! each generate one counter-derived name and perform one explicit creation,
//! leaving publication or cleanup to the caller.
//!
//! Invariants: the workspace is pinned at mount time and lives outside every
//! layer root; workdir temps are never visible entries of any layer.
//!
//! ## References
//!
//! - Linux `ofs->workdir` dentry-ref parity:
//!   <https://elixir.bootlin.com/linux/latest/source/fs/overlayfs/super.c#L663-L803>

#![short_vis_path::add(overlayfs)]

use alloc::format;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::{
    fs::{
        file::{InodeMode, InodeType},
        fs_impls::overlayfs::{fs::OverlayFs, inode::CreateOp},
        utils::NAME_MAX,
        vfs::{
            inode::{Inode, MknodType, RenameMode},
            path::Dentry,
        },
    },
    prelude::*,
};

pub(in super::super) struct WorkdirTemp {
    name: String,
    dentry: Arc<Dentry>,
    kind: InodeType,
}

const TEMP_NAME_SUFFIX_LEN: usize = 16;

pub(in overlayfs) fn workdir_temp_name(target_name: &str, next_temp: &AtomicU64) -> String {
    let suffix = next_temp.fetch_add(1, Ordering::Relaxed);
    const TEMP_NAME_SEPARATORS: usize = 2;
    const TEMP_NAME_TARGET_CAP: usize = NAME_MAX - TEMP_NAME_SEPARATORS - TEMP_NAME_SUFFIX_LEN;
    let target_component = &target_name[..target_name.floor_char_boundary(TEMP_NAME_TARGET_CAP)];
    format!("#{target_component}#{suffix:016x}")
}

impl WorkdirTemp {
    pub(in super::super) fn name(&self) -> &str {
        &self.name
    }

    pub(in super::super) fn kind(&self) -> InodeType {
        self.kind
    }

    pub(in super::super) fn inode(&self) -> &Arc<dyn Inode> {
        self.dentry.inode()
    }

    /// The temp's own dentry, paired with [`Self::inode`] for real-layer setter calls.
    pub(in super::super) fn dentry(&self) -> &Arc<Dentry> {
        &self.dentry
    }

    /// Consumes the handle into `(name, dentry)`; the dentry stays valid after the rename.
    pub(in super::super) fn into_parts(self) -> (String, Arc<Dentry>) {
        (self.name, self.dentry)
    }

    /// Creates one temp for a create-family `op`; any error, including `EEXIST`, propagates.
    pub(in super::super) fn create_from_op(
        workspace: &Arc<Dentry>,
        target_name: &str,
        op: &CreateOp<'_>,
        next_temp: &AtomicU64,
    ) -> Result<Self> {
        let name = workdir_temp_name(target_name, next_temp);
        let dentry = op.create_child_in(workspace, &name)?;
        Ok(Self {
            name,
            dentry,
            kind: op.object_type(),
        })
    }

    /// Creates one temp hard-linking `source` under a counter-derived name.
    pub(in super::super) fn create_from_link(
        workspace: &Arc<Dentry>,
        target_name: &str,
        source: &Arc<Dentry>,
        next_temp: &AtomicU64,
    ) -> Result<Self> {
        let name = workdir_temp_name(target_name, next_temp);
        let dir = workspace.as_dir_dentry_or_err()?;
        dir.link(source, &name)?;
        let dentry = dir.lookup_child(&name)?;
        Ok(Self {
            name,
            dentry,
            kind: source.inode().type_(),
        })
    }

    /// Creates one temp holding a char device `0:0` for use as a whiteout.
    pub(in super::super) fn create_from_char_device(
        workspace: &Arc<Dentry>,
        target_name: &str,
        device: u64,
        next_temp: &AtomicU64,
    ) -> Result<Self> {
        let name = workdir_temp_name(target_name, next_temp);
        let dir = workspace.as_dir_dentry_or_err()?;
        let dentry = dir.mknod(&name, InodeMode::empty(), MknodType::CharDevice(device))?;
        Ok(Self {
            name,
            dentry,
            kind: InodeType::CharDevice,
        })
    }
}

impl OverlayFs {
    pub(in super::super) fn create_workdir_temp(
        &self,
        target_name: &str,
        op: &CreateOp<'_>,
    ) -> Result<WorkdirTemp> {
        let workspace = self.workdir_root_dentry()?;
        let next_temp = self.workdir_temp_counter()?;
        WorkdirTemp::create_from_op(&workspace, target_name, op, next_temp)
    }

    pub(in super::super) fn create_workdir_link_temp(
        &self,
        target_name: &str,
        source: &Arc<Dentry>,
    ) -> Result<WorkdirTemp> {
        let workspace = self.workdir_root_dentry()?;
        let next_temp = self.workdir_temp_counter()?;
        WorkdirTemp::create_from_link(&workspace, target_name, source, next_temp)
    }

    pub(in super::super) fn publish_temp(
        &self,
        temp: &WorkdirTemp,
        upper_parent: &Arc<Dentry>,
        name: &str,
        mode: RenameMode,
    ) -> Result<Arc<Dentry>> {
        let workspace = self.workdir_root_dentry()?;
        workspace.as_dir_dentry_or_err()?.rename(
            temp.name(),
            &upper_parent.as_dir_dentry_or_err()?,
            name,
            mode,
        )?;
        Ok(temp.dentry().clone())
    }

    pub(in super::super) fn cleanup_workdir_temp(
        &self,
        temp_name: &str,
        kind: InodeType,
    ) -> Result<()> {
        let workspace = self.workdir_root_dentry()?;
        let dir = workspace.as_dir_dentry_or_err()?;
        if kind.is_directory() {
            dir.rmdir(temp_name)
        } else {
            dir.unlink(temp_name)
        }
    }

    pub(in super::super) fn workdir_root_dentry(&self) -> Result<Arc<Dentry>> {
        let claim = self.upper_workdir_pair().as_ref().ok_or_else(|| {
            Error::with_message(Errno::EROFS, "the overlay mount has no workdir claim")
        })?;
        Ok(claim.workdir_workspace()?.clone())
    }

    pub(in super::super) fn workdir_temp_counter(&self) -> Result<&AtomicU64> {
        let claim = self.upper_workdir_pair().as_ref().ok_or_else(|| {
            Error::with_message(Errno::EROFS, "the overlay mount has no workdir claim")
        })?;
        Ok(claim.next_temp_counter())
    }
}
