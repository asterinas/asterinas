// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(overlayfs)]
//! The workdir temporary lifecycle.
//!
//! [`WorkdirTemp`] preserves the successful name/inode pair, and
//! [`OverlayFs::create_workdir_temp`] retries only `EEXIST`, regenerating the
//! name for each attempt and leaving publication or cleanup to its caller.
//!
//! ## References
//!
//! - Linux `ofs->workdir` dentry-ref parity:
//!   <https://elixir.bootlin.com/linux/latest/source/fs/overlayfs/super.c#L663-L803>

use alloc::format;

use crate::{
    fs::{
        file::{InodeMode, InodeType},
        fs_impls::overlayfs::{
            inode::OverlayInode, mount::claims::UpperWorkdirClaim, superblock::OverlayFs,
        },
        utils::NAME_MAX,
        vfs::{
            inode::{Inode, MknodType},
            path::Path,
        },
    },
    prelude::*,
};

/// Maps the `mknod` kind request to the overlay-visible object type.
///
/// `MknodType` has no `InodeType` conversion, so this match is the only
/// mapping.
pub(super) fn mknod_object_type(mknod: &MknodType) -> InodeType {
    match mknod {
        MknodType::NamedPipe => InodeType::NamedPipe,
        MknodType::CharDevice(_) => InodeType::CharDevice,
        MknodType::BlockDevice(_) => InodeType::BlockDevice,
    }
}

/// The operation to retry while creating a private workdir temp.
pub(super) enum WorkdirTempRequest<'a> {
    Create {
        kind: InodeType,
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

/// A successful private workdir-temp creation; the handle carries the
/// request-derived [`InodeType`] needed by the kind-aware cleanup dispatcher.
pub(super) struct WorkdirTemp {
    name: String,
    path: Path,
    kind: InodeType,
}

const MAX_WORKDIR_TEMP_CREATE_ATTEMPTS: usize = 8;

/// The fixed-length random hex suffix of a workdir temp name (8 CSPRNG
/// bytes rendered as 16 hex digits).
const TEMP_NAME_RANDOM_SUFFIX_LEN: usize = 16;

/// Generates a uniquely-named workdir temp name for `target_name`.
///
/// Uniqueness comes from a CSPRNG random suffix rather than a serial;
/// `create_workdir_temp` already retries `EEXIST` with a fresh name, so a
/// collision is harmless. The target component is capped so the composite
/// stays within [`crate::fs::utils::NAME_MAX`] for any legal target name.
pub(super) fn workdir_temp_name(target_name: &str) -> String {
    let mut random_bytes = [0u8; 8];
    crate::util::random::getrandom(&mut random_bytes);
    const TEMP_NAME_SEPARATORS: usize = 2;
    const TEMP_NAME_TARGET_CAP: usize =
        NAME_MAX - TEMP_NAME_SEPARATORS - TEMP_NAME_RANDOM_SUFFIX_LEN;
    let target_component = &target_name[..target_name.floor_char_boundary(TEMP_NAME_TARGET_CAP)];
    format!(
        "#{target_component}#{:016x}",
        u64::from_le_bytes(random_bytes)
    )
}

impl WorkdirTemp {
    pub(super) fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn kind(&self) -> InodeType {
        self.kind
    }

    /// Returns the real inode of the staged workdir temp.
    ///
    /// Derived from the dentry-anchored [`Path`], so the inode and the path
    /// always refer to the same workdir object.
    pub(super) fn inode(&self) -> &Arc<dyn Inode> {
        self.path.inode()
    }

    /// Consumes the handle into its `(name, path)` parts; the dentry-anchored
    /// path stays valid after the workdir-to-upper rename and doubles as the
    /// published upper object's path.
    pub(super) fn into_parts(self) -> (String, Path) {
        (self.name, self.path)
    }
}

/// The prepared `<workdir>/work` staging workspace.
#[derive(Debug)]
pub(super) struct WorkdirWorkspace {
    pub(super) path: Path,
}

impl UpperWorkdirClaim {
    pub(super) fn workdir_workspace(&self) -> Result<&Arc<dyn Inode>> {
        self.workdir_workspace
            .as_ref()
            .map(|workspace| workspace.path.inode())
            .ok_or_else(|| {
                Error::with_message(
                    Errno::EROFS,
                    "the overlay workdir workspace is not prepared",
                )
            })
    }

    pub(in overlayfs) fn workdir_workspace_path(&self) -> Result<&Path> {
        self.workdir_workspace
            .as_ref()
            .map(|workspace| &workspace.path)
            .ok_or_else(|| {
                Error::with_message(
                    Errno::EROFS,
                    "the overlay workdir workspace is not prepared",
                )
            })
    }
}

impl WorkdirTempRequest<'_> {
    fn kind(&self) -> InodeType {
        match self {
            Self::Create { kind, .. } => *kind,
            Self::Mknod { node, .. } => mknod_object_type(node),
            Self::Link { source } => source.inode().type_(),
        }
    }

    fn create_in(&self, workdir_path: &Path, temp_name: &str) -> Result<Path> {
        match self {
            Self::Create { kind, mode } => workdir_path.new_fs_child(temp_name, *kind, *mode),
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
                Ok(super::lookup_child_path(workdir_path, temp_name)?)
            }
        }
    }
}

impl OverlayFs {
    /// Creates a private workdir temp object for copy-up staging, retrying
    /// only `EEXIST` with a fresh name and propagating all other errors.
    ///
    /// Staging lives in the workdir workspace; the caller owns publication
    /// and cleanup via the returned handle.
    pub(super) fn create_workdir_temp(
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

    /// Removes a workdir temp object, dispatching on its known kind.
    ///
    /// Directories are removed with `rmdir` and every other kind with
    /// `unlink`, because the underlying filesystem refuses to `unlink` a
    /// directory (`EISDIR`) and would otherwise leak directory-temp residue.
    pub(super) fn cleanup_workdir_temp(&self, temp_name: &str, kind: InodeType) -> Result<()> {
        let workdir_path = self.workdir_root_path()?;
        if kind.is_directory() {
            workdir_path.rmdir(temp_name)
        } else {
            workdir_path.unlink(temp_name)
        }
    }

    /// Resolves the pinned workdir staging workspace path of this writable
    /// mount.
    ///
    /// The path is fixed at mount time and never re-resolves the `work` name;
    /// a missing claim or unprepared workspace means the mount is effectively
    /// read-only, so this entry returns `EROFS` before any workdir side effect.
    pub(super) fn workdir_root_path(&self) -> Result<Path> {
        let claim = self.claims.as_ref().ok_or_else(|| {
            Error::with_message(Errno::EROFS, "the overlay mount has no workdir claim")
        })?;
        Ok(claim.workdir_workspace_path()?.clone())
    }
}

impl OverlayInode {
    /// Returns the pinned workdir staging workspace path of this mount.
    ///
    /// Lets the copy-up recipe arms resolve the staging workspace without
    /// re-upgrading the mount themselves.
    pub(in overlayfs) fn workdir_root_path(&self) -> Result<Path> {
        self.fs_arc()?.workdir_root_path()
    }
}
