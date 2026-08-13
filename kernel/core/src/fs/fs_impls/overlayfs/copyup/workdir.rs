// SPDX-License-Identifier: MPL-2.0

//! The workdir temporary lifecycle.
//!
//! This module owns the workdir-temp retry contract.
//!
//! [`WorkdirTempRequest`] describes the closed set of staging operations and
//! [`WorkdirTemp`] preserves the successful name/inode pair. The
//! [`OverlayFs::create_workdir_temp`] entry retries only `EEXIST`, regenerates
//! the name for every attempt, and leaves publication or cleanup to its caller.
//!
//! The workdir staging workspace (`<workdir>/work`) is a private staging area
//! on the upper filesystem, never a layer: temporaries never enter
//! lookup/readdir, unique naming keeps them out of the overlay namespace, and
//! a failure leaves a cleanup obligation, never a visible entry. A temp
//! handle belongs only to the winner's copy-up transaction: it is never
//! returned to the VFS, never stored on the inode, and never a page-cache
//! forwarding target. The claim protocol guarantees no cross-mount collision
//! (a workdir cannot be claimed by two live mounts), so the composite name
//! needs only per-mount uniqueness.
//!
//! Lock contract: workdir temp naming is uniqueness-based, not lock-based —
//! no Overlay lock is acquired or held by any method here, and the underlying
//! upper-filesystem calls run against that filesystem's own locking (proven
//! non-re-entrant into Overlay). The EROFS gate precedes every workdir/upper
//! side effect: the private [`OverlayFs::workdir_root_path`] resolver
//! returns `Err(Errno::EROFS)` when no writable claim exists or the staging
//! workspace was never prepared.
//!
//! [`OverlayFs::workdir_root_path`] is the dentry-anchored workdir
//! staging-workspace resolver of the overlayfs tree.

use alloc::format;

use crate::{
    fs::{
        file::{InodeMode, InodeType},
        fs_impls::overlayfs::{dir::mknod_object_type, mount::OverlayFs},
        utils::NAME_MAX,
        vfs::{
            inode::{Inode, MknodType},
            path::Path,
        },
    },
    prelude::*,
};

/// The operation to retry while creating a private workdir temp.
pub(in crate::fs::fs_impls::overlayfs) enum WorkdirTempRequest<'a> {
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

/// A successful private workdir-temp creation.
///
/// The handle carries the staged object's [`InodeType`], derived from the
/// request at creation time: the kind-aware cleanup dispatcher
/// ([`OverlayFs::cleanup_workdir_temp`]) needs to know whether the staged
/// object is a directory (`rmdir`) or not (`unlink`), and the kind is a
/// known fact of the request — never a later re-derivation.
pub(in crate::fs::fs_impls::overlayfs) struct WorkdirTemp {
    name: String,
    path: Path,
    kind: InodeType,
}

const MAX_WORKDIR_TEMP_CREATE_ATTEMPTS: usize = 8;

impl WorkdirTemp {
    pub(in crate::fs::fs_impls::overlayfs) fn name(&self) -> &str {
        &self.name
    }

    /// Returns the object kind of the staged workdir temp.
    ///
    /// The kind is the request-derived [`InodeType`] written at creation;
    /// consumers that perform their own best-effort cleanup
    /// ([`OverlayFs::cleanup_workdir_temp`]) pass it through so the cleanup
    /// dispatches on `InodeType::Dir` (`rmdir`) vs everything else
    /// (`unlink`).
    pub(in crate::fs::fs_impls::overlayfs) fn kind(&self) -> InodeType {
        self.kind
    }

    /// Returns the real inode of the staged workdir temp.
    ///
    /// Derived from the dentry-anchored [`Path`], so the inode and the path
    /// always refer to the same workdir object.
    pub(in crate::fs::fs_impls::overlayfs) fn inode(&self) -> &Arc<dyn Inode> {
        self.path.inode()
    }

    /// Consumes the handle into its `(name, path)` parts.
    ///
    /// The dentry-anchored path remains valid after the workdir-to-upper
    /// rename (the same dentry is renamed), so it doubles as the published
    /// upper object's path.
    pub(in crate::fs::fs_impls::overlayfs) fn into_parts(self) -> (String, Path) {
        (self.name, self.path)
    }
}

impl WorkdirTempRequest<'_> {
    /// Returns the object kind of the staged workdir temp.
    ///
    /// A known fact of the request, never a re-derivation: `Create` carries
    /// the kind directly, `Mknod` maps the node kind through the shared
    /// [`mknod_object_type`]
    /// mapping (the single `MknodType` -> `InodeType` classification), and
    /// `Link` inherits the hard-linked source's type. The kind feeds the
    /// [`WorkdirTemp`] handle and the kind-aware cleanup dispatcher.
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
                Ok(Path::new(
                    workdir_path.mount_node().clone(),
                    workdir_path
                        .dentry()
                        .as_dir_dentry_or_err()?
                        .lookup_child(temp_name)?,
                ))
            }
        }
    }
}

impl OverlayFs {
    /// Generates a uniquely-named workdir temp name for a copy-up target.
    ///
    /// The composite is `#{target_name}#{parent_ino}#{serial}`: the target's
    /// publication name, the upper-parent real inode number ([`Inode::ino`]),
    /// and one per-mount saturating workdir serial
    /// ([`OverlayFs::workdir_temp_serial`]). The target-name component is
    /// capped so the composite stays within [`crate::fs::utils::NAME_MAX`]
    /// for any legal target name. The retry entry regenerates the name before
    /// each attempt as the collision backstop.
    pub(in crate::fs::fs_impls::overlayfs) fn generate_workdir_temp_name(
        &self,
        target_name: &str,
        upper_parent: &Path,
    ) -> String {
        let parent_ino = upper_parent.inode().ino();
        let serial = self.workdir_temp_serial();
        const TEMP_NAME_SEPARATORS: usize = 3;
        const U64_DEC_DIGITS_MAX: usize = 20;
        const TEMP_NAME_FIXED_OVERHEAD: usize = TEMP_NAME_SEPARATORS + 2 * U64_DEC_DIGITS_MAX;
        const TEMP_NAME_TARGET_CAP: usize = NAME_MAX - TEMP_NAME_FIXED_OVERHEAD;
        let target_component =
            &target_name[..target_name.floor_char_boundary(TEMP_NAME_TARGET_CAP)];
        format!("#{target_component}#{parent_ino}#{serial}")
    }

    /// Creates a private workdir temp object for copy-up staging.
    ///
    /// Each attempt generates a fresh name and dispatches the same typed
    /// request. Only `EEXIST` retries; on exhaustion the final underlying
    /// `EEXIST` is returned, while all other errors propagate unchanged.
    pub(in crate::fs::fs_impls::overlayfs) fn create_workdir_temp(
        &self,
        target_name: &str,
        upper_parent_path: &Path,
        request: WorkdirTempRequest<'_>,
    ) -> Result<WorkdirTemp> {
        let workdir_path = self.workdir_root_path()?;
        let mut final_eexist = None;

        for _ in 0..MAX_WORKDIR_TEMP_CREATE_ATTEMPTS {
            let name = self.generate_workdir_temp_name(target_name, upper_parent_path);
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
    /// A directory temp (a staged directory copy-up or the clear-empty
    /// staging directory) is removed with `rmdir`; every other object kind
    /// is removed with `unlink` — the underlying filesystem refuses to
    /// `unlink` a directory (`EISDIR`), so without the kind dispatch a
    /// pre-commit failure of a directory temp would leak residue in the
    /// workdir. The kind is supplied by the caller from the request-derived
    /// [`WorkdirTemp::kind`] (a known fact, never a re-derivation). The
    /// recipe calls this best-effort on any pre-publication failure; a
    /// cleanup failure propagates as a known workdir-cleanup debt and never
    /// becomes a visible namespace entry.
    pub(in crate::fs::fs_impls::overlayfs) fn cleanup_workdir_temp(
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

    /// Resolves the pinned workdir staging workspace path of this writable
    /// mount.
    ///
    /// The dentry-anchored workdir staging-workspace resolver of the
    /// overlayfs tree: every dentry-routed workdir consumer — the temp
    /// lifecycle helpers in this file and `OverlayInode::workdir_root_path`
    /// (`copyup/promote.rs`) — funnels through this one entry, so the
    /// claim-resolution shape and the EROFS error text exist exactly once.
    /// The workspace path is pinned on the claim by
    /// `UpperWorkdirClaim::prepare_workdir` during mount construction (Linux
    /// `ofs->workdir` dentry-ref parity: staging never re-resolves the `work`
    /// name); the claim is reachable via `claims()`. A missing claim or an
    /// unprepared workspace means the mount is effectively read-only (or the
    /// claims were released), so the EROFS gate fires here — before any
    /// workdir/upper side effect.
    pub(in crate::fs::fs_impls::overlayfs) fn workdir_root_path(&self) -> Result<Path> {
        let claim = self.claims().ok_or_else(|| {
            Error::with_message(Errno::EROFS, "the overlay mount has no workdir claim")
        })?;
        Ok(claim.workdir_workspace_path()?.clone())
    }
}
