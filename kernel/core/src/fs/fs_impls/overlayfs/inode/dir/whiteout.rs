// SPDX-License-Identifier: MPL-2.0

//! This module owns the whiteout concept in both directions: what a whiteout is
//! ([`OverlayFs::is_whiteout`] as the confirmed read over the two physical forms — a char device
//! `0:0` or a zero-size regular file carrying the record), and how one is published, removed, and
//! swept ([`WhiteoutCache`] and its [`WhiteoutHandle`], `OverlayFs::publish_whiteout`,
//! `OverlayFs::remove_whiteout`, `OverlayFs::sweep_whiteouts`).
//!
//! The two directions are deliberately asymmetric: the write side picks a form from this mount's
//! capability, while the read side answers the union of both forms at every layer — a lower
//! layer's whiteout was published by another mount, so reading never consults this one's
//! capability.
//!
//! Invariants: at most one shared whiteout (a workdir object, never a visible
//! entry) and it is only borrowed, never taken; `can_share_by_link` is set
//! once and never re-enabled; a published whiteout is a visibility barrier,
//! never an inode.

#![short_vis_path::add(overlayfs)]

use core::sync::atomic::{AtomicBool, Ordering};

use crate::{
    fs::{
        file::{InodeMode, InodeType},
        fs_impls::overlayfs::{
            fs::{
                OverlayFs,
                mount::{WhiteoutCapability, inuse::UpperWorkdirInuse},
            },
            inode::{CreateOp, copyup::workdir::WorkdirTemp, xattr::OverlayXattrType},
            real::read_child_names,
        },
        vfs::{
            inode::{MknodType, RenameMode},
            path::Dentry,
            xattr::XattrNamespace,
        },
    },
    prelude::*,
};

#[derive(Debug)]
pub(in overlayfs) struct WhiteoutCache {
    /// Holds the mount-time shared whiteout, borrowed for link publishes; `None` when unavailable.
    ///
    /// The cache never cleans it up: removing this object is the workdir's mount/unmount duty, not
    /// this holder's.
    shared: Option<WhiteoutHandle>,
    /// Holds a one-way link-capability latch: a capability error flips it once and never back.
    can_share_by_link: AtomicBool,
}

impl WhiteoutCache {
    pub(in overlayfs) fn new() -> Self {
        Self {
            shared: None,
            can_share_by_link: AtomicBool::new(true),
        }
    }

    /// Creates one shared workspace whiteout when the capability selects a form.
    pub(in overlayfs) fn with_shared_whiteout(
        capability: WhiteoutCapability,
        upper_workdir: &UpperWorkdirInuse,
        namespace: XattrNamespace,
    ) -> Self {
        let shared = upper_workdir.stage_whiteout(capability, namespace).ok();
        Self {
            shared,
            can_share_by_link: AtomicBool::new(true),
        }
    }

    /// Returns `Some` only while sharing is both enabled and materialized.
    fn shared_whiteout(&self) -> Option<&WhiteoutHandle> {
        if self.can_share_by_link.load(Ordering::Relaxed) {
            self.shared.as_ref()
        } else {
            None
        }
    }

    fn latch_degraded(&self) {
        self.can_share_by_link.store(false, Ordering::Relaxed);
    }
}

/// One shared workspace whiteout, detached from the temp guard that created it and held for the
/// whole mount.
///
/// The detach is the point: the creating guard's cleanup must never apply to this one, the cache
/// only ever borrows it (`WhiteoutCache::shared_whiteout`) without taking it, and the object
/// stays a zero-size regular file (or a `0:0` char device) for as long as the mount lives, which
/// is what the read side's xattr form recognizes.
#[derive(Debug)]
struct WhiteoutHandle {
    workdir_name: String,
    dentry: Arc<Dentry>,
}

impl UpperWorkdirInuse {
    /// Stages one whiteout object in the workspace under the form `capability` selects and
    /// detaches it into a handle the workspace never cleans up.
    fn stage_whiteout(
        &self,
        capability: WhiteoutCapability,
        namespace: XattrNamespace,
    ) -> Result<WhiteoutHandle> {
        /// The staged-name component every whiteout temp is created under.
        const WHITEOUT_TEMP_NAME_COMPONENT: &str = "whiteout";
        match capability {
            WhiteoutCapability::CharDevice => {
                /// The device number of the char-device whiteout: `0:0`, the identity every mount reads as one.
                ///
                /// Linux publishes the same device identity for it.
                ///
                /// Reference:
                /// <https://elixir.bootlin.com/linux/v6.17/source/fs/overlayfs/overlayfs.h#L52>.
                const WHITEOUT_CHAR_DEV: u64 = 0;
                let temp = self.create_workdir_temp(
                    WHITEOUT_TEMP_NAME_COMPONENT,
                    CreateOp::Mknod {
                        mode: InodeMode::empty(),
                        node: &MknodType::CharDevice(WHITEOUT_CHAR_DEV),
                    },
                )?;
                Ok(temp.into_whiteout())
            }
            WhiteoutCapability::Xattr => {
                // No xattr capability gate is needed: this arm is only reached under `Xattr`.
                // A zero-size regular file carrying the record is the form Linux checks for one
                // (`ovl_path_check_xwhiteout_xattr`).
                //
                // Reference:
                // <https://elixir.bootlin.com/linux/v6.17/source/fs/overlayfs/util.c#L772-L783>.
                let temp = self.create_workdir_temp(
                    WHITEOUT_TEMP_NAME_COMPONENT,
                    CreateOp::general(InodeType::File, InodeMode::empty()),
                )?;
                // A failed record leaves the guard unconsumed, so `Drop` removes the temp: the
                // hand-written unlink is subsumed by it, which picks the op by its own type.
                OverlayXattrType::Whiteout.set_value_on(temp.dentry(), namespace, None)?;
                Ok(temp.into_whiteout())
            }
            WhiteoutCapability::Unsupported => Err(Error::with_message(
                Errno::EOPNOTSUPP,
                "the upper filesystem supports no whiteout form",
            )),
        }
    }
}

impl WorkdirTemp {
    /// Consumes this staged temp into the whiteout handle that borrows or publishes it.
    fn into_whiteout(mut self) -> WhiteoutHandle {
        self.mark_consumed();
        WhiteoutHandle {
            workdir_name: self.name().into(),
            dentry: self.dentry().clone(),
        }
    }
}

impl OverlayFs {
    /// Whether `real_dentry` is a whiteout: the union of the char-device form and the xattr form,
    /// both published by whichever mount wrote this layer.
    ///
    /// A lower layer's whiteout comes from another mount, so the read never consults this mount's
    /// capability. The xattr form is a whiteout **only** on a zero-size regular file, and the
    /// record counts by its presence alone: the stored bytes are never compared. The answer reuses
    /// the metadata already read for the char-device arm, so no extra real-layer call is made.
    pub(in super::super) fn is_whiteout(&self, real_dentry: &Dentry) -> Result<bool> {
        let metadata = real_dentry.inode().metadata()?;
        if metadata.type_ == InodeType::CharDevice
            && metadata.self_dev_id.is_none_or(|dev_id| dev_id.is_null())
        {
            return Ok(true);
        }
        if metadata.type_ != InodeType::File || metadata.size != 0 {
            return Ok(false);
        }
        OverlayXattrType::Whiteout.is_positive_on(real_dentry, self.policy().xattr_namespace())
    }

    pub(super) fn publish_whiteout(
        &self,
        upper_parent: &Arc<Dentry>,
        name: &str,
        replace_target: Option<InodeType>,
    ) -> Result<()> {
        if replace_target.is_none()
            && let Some(shared) = self.whiteout_cache().shared_whiteout()
        {
            let parent_dir = upper_parent.as_dir_dentry_or_err()?;
            match parent_dir.link(&shared.dentry, name) {
                Ok(()) => return Ok(()),
                Err(err) => {
                    if matches!(err.error(), Errno::EMLINK | Errno::EOPNOTSUPP) {
                        self.whiteout_cache().latch_degraded();
                    }
                    // Falls back to the one-shot rename, which still hides the name.
                }
            }
        }

        // One-shot temp: create, use, never cache.
        let handle = self.create_whiteout_temp()?;
        let workspace = self.upper_workdir_inuse().workdir_workspace()?;
        let workdir_dir = workspace.as_dir_dentry_or_err()?;
        let upper_dir = upper_parent.as_dir_dentry_or_err()?;
        match replace_target {
            Some(target_type) if target_type.is_directory() => {
                workdir_dir.rename(&handle.workdir_name, &upper_dir, name, RenameMode::Exchange)?;
                if let Err(err) = workdir_dir.rmdir(&handle.workdir_name) {
                    warn!(
                        "whiteout publish: workdir cleanup of the displaced directory {:?} failed: {:?}",
                        handle.workdir_name, err
                    );
                }
            }
            _ => {
                workdir_dir.rename(&handle.workdir_name, &upper_dir, name, RenameMode::Replace)?;
            }
        }
        Ok(())
    }

    fn create_whiteout_temp(&self) -> Result<WhiteoutHandle> {
        // `Unsupported` is a hard error: a new whiteout cannot be represented.
        self.upper_workdir_inuse().stage_whiteout(
            self.policy().whiteout_capability(),
            self.policy().xattr_namespace(),
        )
    }

    /// Removes the displaced whiteout the exchange left at `name` in the upper parent.
    ///
    /// A name that no longer holds a whiteout is `EIO`: the exchange put one there, so a missing
    /// one means the entry was replaced behind this removal's back. A non-conforming object that
    /// merely carries the record is not a whiteout and answers `EIO` the same way.
    pub(super) fn remove_whiteout(&self, upper_parent: &Arc<Dentry>, name: &str) -> Result<()> {
        let upper_dir = upper_parent.as_dir_dentry_or_err()?;
        let holds_whiteout = match upper_dir.lookup_child(name) {
            Ok(child) => self.is_whiteout(&child)?,
            Err(err) if err.error() == Errno::ENOENT => false,
            Err(err) => return Err(err),
        };
        if !holds_whiteout {
            return Err(Error::with_message(
                Errno::EIO,
                "the cleared name no longer holds the whiteout the exchange displaced",
            ));
        }
        upper_dir.unlink(name)?;
        Ok(())
    }

    /// Sweeps physical whiteout residue out of an upper directory before the physical
    /// `rmdir`/`rename`.
    ///
    /// Non-atomic and pre-commit: a failure aborts the caller and a retry converges; it never
    /// recurses into directories and propagates errors unchanged. The full pass runs before any
    /// entry is deleted, so an `ENOTEMPTY` answer leaves the directory untouched. The whiteouts
    /// it clears are the **upper's** own: only this mount publishes there, and an external
    /// writer under it is outside the supported contract.
    pub(super) fn sweep_whiteouts(&self, upper_dir: &Arc<Dentry>) -> Result<()> {
        let names = read_child_names(upper_dir.inode())?;
        self.validate_whiteout_children(upper_dir, &names)?;
        let dir = upper_dir.as_dir_dentry_or_err()?;
        for name in &names {
            dir.unlink(name)?;
        }
        Ok(())
    }

    /// Refuses the sweep when any child of `upper_dir` is not a whiteout, before anything is deleted.
    fn validate_whiteout_children(&self, upper_dir: &Arc<Dentry>, names: &[String]) -> Result<()> {
        let dir = upper_dir.as_dir_dentry_or_err()?;
        for name in names {
            let child = dir.lookup_child(name)?;
            if !self.is_whiteout(&child)? {
                return Err(Error::with_message(
                    Errno::ENOTEMPTY,
                    "a hidden non-whiteout entry prevents the overlay directory removal",
                ));
            }
        }
        Ok(())
    }
}
