// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(overlayfs)]
//! The shared whiteout cache and whiteout-publish mechanics.
//!
//! This module owns [`WhiteoutCache`] (the one-slot shared cache),
//! [`WhiteoutHandle`] (a cached or mutation-local workdir whiteout), and
//! [`WhiteoutRepresentation`] (the char-device or xattr whiteout form).
//!
//! Invariants: at most one cached whiteout (a workdir object, never a
//! visible entry); `can_share_by_link` is set once and never re-enabled;
//! a published whiteout is a visibility barrier, never an inode.
//!
//! ## References
//!
//! - <https://elixir.bootlin.com/linux/v6.17/source/fs/overlayfs/overlayfs.h#L52>
//!   (Linux whiteout device identity)
//! - <https://elixir.bootlin.com/linux/v6.17/source/fs/overlayfs/dir.c#L81-L129>
//!   (Linux `ovl_whiteout` whiteout creation)
//! - <https://elixir.bootlin.com/linux/v6.17/source/fs/overlayfs/readdir.c#L989-L1030>
//!   (Linux `ovl_check_empty_dir` whiteout sweep)

use crate::{
    fs::{
        file::{InodeMode, InodeType},
        fs_impls::overlayfs::{
            metadata_security::xattr::{
                WHITEOUT_MARKER_VALUE, WHITEOUT_XATTR_FULL_NAME, XattrPolicy,
            },
            projection::is_whiteout_inode,
            superblock::OverlayFs,
            workdir::WorkdirTempRequest,
        },
        vfs::{
            inode::{Inode, MknodType, RenameMode},
            path::Path,
            xattr::XattrSetFlags,
        },
    },
    prelude::*,
};

/// The classic-whiteout char device `0:0`.
///
/// The device number `0` is the `makedev(0, 0)` encoding of the kernel's
/// whiteout device identity. The whiteout reader is presence-based and never
/// inspects the number, but the char-device whiteout form is exactly this
/// contract.
const WHITEOUT_CHAR_DEV: u64 = 0;

const WHITEOUT_TEMP_NAME_COMPONENT: &str = "whiteout";

/// A `Mutex` rather than an `RwMutex` because the critical sections are
/// short slot operations with no read-mostly workload.
#[derive(Debug)]
pub(in overlayfs) struct WhiteoutCache {
    cached: Option<WhiteoutHandle>,
    can_share_by_link: bool,
}

impl WhiteoutCache {
    pub(in overlayfs) fn new() -> Self {
        Self {
            cached: None,
            can_share_by_link: true,
        }
    }

    fn take(&mut self) -> Option<WhiteoutHandle> {
        self.cached.take()
    }

    /// Pushes a whiteout handle back into the cache's single slot.
    ///
    /// Bounded to one slot: an occupied slot is a protocol violation, so the
    /// stale handle is dropped (workdir-cleanup residue, never a visible
    /// source) rather than exceeding the bound.
    fn store(&mut self, handle: WhiteoutHandle) {
        if self.cached.replace(handle).is_some() {
            warn!(
                "overlay whiteout cache slot occupied at store; the stale cached whiteout is \
                 dropped (workdir-cleanup residue, never a visible source)"
            );
        }
    }

    /// Disables whiteout sharing by link (the cache's fallback flag).
    ///
    /// Set on `EMLINK`/`EOPNOTSUPP` from the link path; once `false`, every
    /// future publish uses rename-over move semantics.
    fn disable_sharing(&mut self) {
        self.can_share_by_link = false;
    }
}

/// One cached or mutation-local workdir whiteout.
///
/// Invariants: `workdir_name` is non-empty and unique; the handle never
/// outlives its use in one mutation unless re-cached. Owned by
/// `WhiteoutCache::cached` or a mutation-local.
#[derive(Debug)]
pub(super) struct WhiteoutHandle {
    /// The whiteout object (char `0:0` device or zero-size file + whiteout
    /// xattr).
    #[expect(
        dead_code,
        reason = "retained strong pin: the strong inode pin keeps the workdir object alive \
                  while the dentry-anchored `path` routes the publish arms"
    )]
    inode: Arc<dyn Inode>,
    /// Its name in the workdir; needed for rename-over publishes.
    workdir_name: String,
    /// The dentry-anchored workdir temp path of the whiteout; the
    /// `Path::link`/`Path::rename` publish arms route through it.
    path: Path,
}

/// The physical whiteout forms, classified as an enum rather than a bool
/// because the two forms carry different recipe behavior (mknod vs
/// create+xattr).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WhiteoutRepresentation {
    /// Classic whiteout: char device `0:0` (workdir mknod).
    CharDevice,
    /// Xattr whiteout: zero-size regular file + `trusted.overlay.whiteout`
    /// (needs `can_store_private_xattr`).
    Xattr,
}

impl OverlayFs {
    /// Derives the whiteout representation from the published capabilities;
    /// a missing capability snapshot is `EROFS` and an unsupported upper is
    /// the defensive `EOPNOTSUPP`.
    fn whiteout_representation(&self) -> Result<WhiteoutRepresentation> {
        let capabilities = self.policy.upper_capabilities().ok_or_else(|| {
            Error::with_message(
                Errno::EROFS,
                "the overlay mount has no writable upper capability snapshot",
            )
        })?;
        if capabilities.can_mknod_char() {
            Ok(WhiteoutRepresentation::CharDevice)
        } else if capabilities.can_store_private_xattr() {
            Ok(WhiteoutRepresentation::Xattr)
        } else {
            Err(Error::with_message(
                Errno::EOPNOTSUPP,
                "the upper filesystem supports no whiteout form (neither char-device mknod \
                 nor private xattr)",
            ))
        }
    }

    /// Creates one private workdir whiteout temp without touching the
    /// whiteout cache lock.
    ///
    /// The temp is created in the workdir (never a visible source); on a
    /// failure the created temp is removed best-effort so no workdir residue
    /// outlives the failed creation.
    fn create_whiteout_temp(&self) -> Result<WhiteoutHandle> {
        let representation = self.whiteout_representation()?;
        let (workdir_name, path) = match representation {
            WhiteoutRepresentation::CharDevice => {
                let node = MknodType::CharDevice(WHITEOUT_CHAR_DEV);
                self.create_workdir_temp(
                    WHITEOUT_TEMP_NAME_COMPONENT,
                    WorkdirTempRequest::Mknod {
                        mode: InodeMode::empty(),
                        node: &node,
                    },
                )?
                .into_parts()
            }
            WhiteoutRepresentation::Xattr => {
                // The representation derivation already gated this branch on
                // `can_store_private_xattr`.
                debug_assert!(
                    self.xattr_policy.is_private(WHITEOUT_XATTR_FULL_NAME),
                    "the whiteout marker name must classify as an overlay-private record"
                );
                let temp = self.create_workdir_temp(
                    WHITEOUT_TEMP_NAME_COMPONENT,
                    WorkdirTempRequest::Create {
                        kind: InodeType::File,
                        mode: InodeMode::empty(),
                    },
                )?;
                let marker_name = XattrPolicy::whiteout_marker_name()?;
                let mut marker_reader = VmReader::from(WHITEOUT_MARKER_VALUE).to_fallible();
                if let Err(err) = temp.inode().set_xattr(
                    marker_name,
                    &mut marker_reader,
                    XattrSetFlags::CREATE_OR_REPLACE,
                ) {
                    // Best-effort temp cleanup on the pre-publication failure
                    // (the cleanup debt never becomes a visible entry).
                    let _ = self.cleanup_workdir_temp(temp.name(), temp.kind());
                    return Err(err);
                }
                temp.into_parts()
            }
        };
        Ok(WhiteoutHandle {
            inode: path.inode().clone(),
            workdir_name,
            path,
        })
    }

    /// Publishes a whiteout at `(upper_parent_path, name)`.
    ///
    /// `None` publishes by link and re-caches the shared workdir original
    /// (`EMLINK`/`EOPNOTSUPP` degrade to rename-over move semantics);
    /// `Some(non-dir)` publishes by `Replace`, consuming the whiteout;
    /// `Some(Dir)` publishes by `Exchange`, cleaning up the displaced
    /// directory best-effort. The whiteout marker is written at temp
    /// creation, before the link/rename.
    pub(super) fn publish_whiteout(
        &self,
        upper_parent_path: &Path,
        name: &str,
        replace_target: Option<InodeType>,
    ) -> Result<()> {
        let (cached, can_share_by_link) = {
            let mut cache = self.whiteout_cache.lock();
            let cached = cache.take();
            let can_share_by_link = cache.can_share_by_link;
            (cached, can_share_by_link)
        };

        let handle = match cached {
            Some(handle) => handle,
            None => self.create_whiteout_temp()?,
        };

        let workdir_path = self.workdir_root_path()?;
        // Publishing a whiteout inside the parent makes it impure, so the
        // marker is refreshed before the physical publish. The marker is a
        // cache hint whose consumer refreshes it best-effort, so a marker
        // failure must not abort the physical publish: warn and continue.
        if let Err(err) = self
            .xattr_policy
            .set_impure_marker(upper_parent_path.inode())
        {
            warn!(
                "overlay whiteout publish: failed to set the impure marker on {:?} \
                 (best-effort cache hint; continuing with the physical publish): {:?}",
                upper_parent_path.inode(),
                err
            );
        }
        match replace_target {
            None => {
                if can_share_by_link {
                    match upper_parent_path.link(&handle.path, name) {
                        Ok(()) => {
                            self.whiteout_cache.lock().store(handle);
                            return Ok(());
                        }
                        Err(err) if matches!(err.error(), Errno::EMLINK | Errno::EOPNOTSUPP) => {
                            self.whiteout_cache.lock().disable_sharing();
                        }
                        Err(err) => return Err(err),
                    }
                }
            }
            Some(target_type) if !target_type.is_directory() => {}
            Some(_) => {
                workdir_path.rename(
                    &handle.workdir_name,
                    upper_parent_path,
                    name,
                    RenameMode::Exchange,
                )?;
                if let Err(cleanup_err) = workdir_path.rmdir(&handle.workdir_name) {
                    warn!(
                        "overlay whiteout publish: workdir cleanup of the displaced directory \
                         {:?} failed (residue, never a visible source): {:?}",
                        handle.workdir_name, cleanup_err
                    );
                }
                return Ok(());
            }
        }
        workdir_path.rename(
            &handle.workdir_name,
            upper_parent_path,
            name,
            RenameMode::Replace,
        )?;
        Ok(())
    }
}

/// Sweeps physical whiteout residue out of an upper directory before the
/// physical rmdir/rename.
///
/// Non-atomic and pre-commit: a failure aborts the removal and a retry
/// converges; it never recurses into directories and propagates errors
/// unchanged.
pub(super) fn cleanup_upper_whiteouts(upper_dir_path: &Path) -> Result<()> {
    let names = crate::fs::fs_impls::overlayfs::read_child_names(upper_dir_path.inode())?;
    validate_whiteout_children(upper_dir_path, &names)?;
    unlink_rechecked_whiteouts(upper_dir_path, &names)?;
    Ok(())
}

/// Returns whether the named physical child of `upper_dir_path` is a whiteout.
///
/// The child is re-observed through the base VFS `Path` layer (`lookup_child`)
/// and classified with the shared [`is_whiteout_inode`] predicate, keeping
/// the base view's `DentryChildren` coherent. Underlying lookup errors
/// propagate unchanged.
fn is_whiteout_child(upper_dir_path: &Path, name: &str) -> Result<bool> {
    let child_path = super::super::lookup_child_path(upper_dir_path, name)?;
    is_whiteout_inode(child_path.inode())
}

/// Validates that every named physical child of `upper_dir_path` is a
/// whiteout, removing nothing.
///
/// The full-validation pass of the sweep: any non-whiteout child returns
/// `ENOTEMPTY`, so the sweep refuses the removal before any entry is deleted.
fn validate_whiteout_children(upper_dir_path: &Path, names: &[String]) -> Result<()> {
    for name in names {
        if !is_whiteout_child(upper_dir_path, name)? {
            return Err(Error::with_message(
                Errno::ENOTEMPTY,
                "a hidden non-whiteout entry prevents the overlay directory removal",
            ));
        }
    }
    Ok(())
}

/// Re-observes and unlinks every named whiteout child of `upper_dir_path`.
///
/// The removal pass of the sweep: each child is re-classified immediately
/// before its `unlink`, so an entry swapped in since the validation pass is
/// refused (`ENOTEMPTY`) instead of deleted. The re-check narrows but cannot
/// close the residual check-to-use window, so the upper directory must not be
/// modified concurrently.
fn unlink_rechecked_whiteouts(upper_dir_path: &Path, names: &[String]) -> Result<()> {
    for name in names {
        if !is_whiteout_child(upper_dir_path, name)? {
            return Err(Error::with_message(
                Errno::ENOTEMPTY,
                "a hidden non-whiteout entry prevents the overlay directory removal",
            ));
        }
        upper_dir_path.unlink(name)?;
    }
    Ok(())
}
