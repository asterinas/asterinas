// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(overlayfs)]
//! The workdir temporary lifecycle.
//!
//! [`WorkdirTemp`] preserves the created name, its dentry-anchored path, and
//! the request-derived kind, and [`OverlayFs::create_workdir_temp`] retries
//! only `EEXIST`, regenerating the name for each attempt and leaving
//! publication or cleanup to its caller.
//!
//! Invariants: the workspace is pinned at mount time and lives outside every
//! layer root; workdir temps are never visible entries of any layer.
//!
//! ## References
//!
//! - Linux `ofs->workdir` dentry-ref parity:
//!   <https://elixir.bootlin.com/linux/latest/source/fs/overlayfs/super.c#L663-L803>

use crate::{
    fs::{
        file::{InodeMode, InodeType},
        fs_impls::overlayfs::{
            fs::OverlayFs,
            inode::{OverlayInode, mknod_object_type},
            workdir_temp_name,
        },
        vfs::{
            inode::{Inode, MknodType, RenameMode},
            path::{Dentry, Path},
        },
    },
    prelude::*,
};

pub(in super::super) enum WorkdirTempRequest<'a> {
    Create {
        kind: InodeType,
        mode: InodeMode,
    },
    /// A symlink temp is created atomically with its target; the mode is
    /// carried so the copy-up publishes the lower's mode unchanged.
    Symlink {
        target: String,
        mode: InodeMode,
    },
    Mknod {
        mode: InodeMode,
        node: &'a MknodType,
    },
    Link {
        source: Path,
    },
}

pub(in super::super) struct WorkdirTemp {
    name: String,
    path: Path,
    kind: InodeType,
}

const MAX_WORKDIR_TEMP_CREATE_ATTEMPTS: usize = 8;

impl WorkdirTemp {
    pub(in super::super) fn name(&self) -> &str {
        &self.name
    }

    pub(in super::super) fn kind(&self) -> InodeType {
        self.kind
    }

    pub(in super::super) fn inode(&self) -> &Arc<dyn Inode> {
        self.path.inode()
    }

    /// The temp's own dentry, paired with [`Self::inode`] for the real-layer
    /// setter calls that take a `(self_dentry, …)` pair.
    pub(in super::super) fn dentry(&self) -> &Arc<Dentry> {
        self.path.dentry()
    }

    fn path(&self) -> &Path {
        &self.path
    }

    /// Consumes the handle into its `(name, path)` parts; the dentry-anchored
    /// path stays valid after the workdir-to-upper rename and doubles as the
    /// published upper object's path.
    pub(in super::super) fn into_parts(self) -> (String, Path) {
        (self.name, self.path)
    }
}

impl WorkdirTempRequest<'_> {
    fn kind(&self) -> InodeType {
        match self {
            Self::Create { kind, .. } => *kind,
            Self::Symlink { .. } => InodeType::SymLink,
            Self::Mknod { node, .. } => mknod_object_type(node),
            Self::Link { source } => source.inode().type_(),
        }
    }

    fn create_in(&self, workdir_path: &Path, temp_name: &str) -> Result<Path> {
        match self {
            Self::Create { kind, mode } => workdir_path.new_child(temp_name, *kind, *mode),
            Self::Symlink { target, mode } => {
                workdir_path.new_symlink_child(temp_name, target, *mode)
            }
            Self::Mknod { mode, node } => {
                let node = match node {
                    MknodType::NamedPipe => MknodType::NamedPipe,
                    MknodType::CharDevice(device_id) => MknodType::CharDevice(*device_id),
                    MknodType::BlockDevice(device_id) => MknodType::BlockDevice(*device_id),
                };
                workdir_path.mknod(temp_name, *mode, node)
            }
            Self::Link { source } => {
                workdir_path.link(source, temp_name)?;
                Ok(super::super::super::lookup_child_path(
                    workdir_path,
                    temp_name,
                )?)
            }
        }
    }
}

impl OverlayFs {
    pub(in super::super) fn create_workdir_temp(
        &self,
        target_name: &str,
        request: WorkdirTempRequest<'_>,
    ) -> Result<WorkdirTemp> {
        let workdir_path = self.workdir_root_path()?;
        let mut final_eexist = None;

        for _ in 0..MAX_WORKDIR_TEMP_CREATE_ATTEMPTS {
            let name = workdir_temp_name(target_name);
            match request.create_in(&workdir_path, &name) {
                Ok(path) => {
                    return Ok(WorkdirTemp {
                        name,
                        path,
                        kind: request.kind(),
                    });
                }
                Err(err) if err.error() == Errno::EEXIST => final_eexist = Some(err),
                Err(err) => return Err(err),
            }
        }

        match final_eexist {
            Some(err) => Err(err),
            None => unreachable!("the nonzero retry bound must attempt workdir creation"),
        }
    }

    pub(in super::super) fn publish_temp(
        &self,
        temp: &WorkdirTemp,
        upper_parent_path: &Path,
        name: &str,
        mode: RenameMode,
    ) -> Result<Path> {
        let workdir_path = self.workdir_root_path()?;
        workdir_path.rename(temp.name(), upper_parent_path, name, mode)?;
        Ok(temp.path().clone())
    }

    pub(in super::super) fn cleanup_workdir_temp(
        &self,
        temp_name: &str,
        kind: InodeType,
    ) -> Result<()> {
        let workdir_path = self.workdir_root_path()?;
        if kind.is_directory() {
            workdir_path.rmdir(temp_name)
        } else {
            workdir_path.unlink(temp_name)
        }
    }

    pub(in super::super) fn workdir_root_path(&self) -> Result<Path> {
        let claim = self.upper_workdir_pair().as_ref().ok_or_else(|| {
            Error::with_message(Errno::EROFS, "the overlay mount has no workdir claim")
        })?;
        Ok(claim.workdir_workspace_path()?.clone())
    }
}

impl OverlayInode {
    pub(in super::super) fn workdir_root_path(&self) -> Result<Path> {
        self.fs_arc()?.workdir_root_path()
    }
}
