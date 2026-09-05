// SPDX-License-Identifier: MPL-2.0

//! The shared whiteout handle and whiteout-publish mechanics.
//!
//! This module owns [`WhiteoutCache`] (the mount-time shared handle plus the
//! one-way link-capability latch), [`WhiteoutHandle`] (a shared or
//! mutation-local workdir whiteout), and [`WhiteoutRepresentation`] (the
//! char-device or xattr whiteout form).
//!
//! Invariants: at most one shared whiteout (a workdir object, never a visible
//! entry) and it is only borrowed, never taken; `can_share_by_link` is set
//! once and never re-enabled; a published whiteout is a visibility barrier,
//! never an inode.
//!
//! ## References
//!
//! - <https://elixir.bootlin.com/linux/v6.17/source/fs/overlayfs/overlayfs.h#L52>
//!   (Linux whiteout device identity)
//! - <https://elixir.bootlin.com/linux/v6.17/source/fs/overlayfs/dir.c#L81-L129>
//!   (Linux `ovl_whiteout` whiteout creation)
//! - <https://elixir.bootlin.com/linux/v6.17/source/fs/overlayfs/readdir.c#L989-L1030>
//!   (Linux `ovl_check_empty_dir` whiteout sweep)

#![short_vis_path::add(overlayfs)]

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::{
    fs::{
        file::{InodeMode, InodeType},
        fs_impls::overlayfs::{
            fs::{OverlayFs, mount::WhiteoutCapability},
            inode::{
                CreateOp, OverlayInode,
                copyup::workdir::WorkdirTemp,
                is_whiteout_inode,
                xattr::{MARKER_VALUE, OverlayRecordName},
            },
        },
        vfs::{
            inode::RenameMode,
            path::Dentry,
            xattr::{XattrNamespace, XattrSetFlags},
        },
    },
    prelude::*,
};

const WHITEOUT_CHAR_DEV: u64 = 0;

const WHITEOUT_TEMP_NAME_COMPONENT: &str = "whiteout";

#[derive(Debug)]
pub(in overlayfs) struct WhiteoutCache {
    /// The mount-time shared whiteout, borrowed for link publishes; `None` when unavailable.
    shared: Option<WhiteoutHandle>,
    /// One-way link-capability latch: a capability error flips it once and never back.
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
        workspace: &Arc<Dentry>,
        namespace: XattrNamespace,
        next_temp: &AtomicU64,
    ) -> Self {
        let shared = match whiteout_representation(capability) {
            Ok(representation) => {
                create_whiteout_handle(workspace, representation, namespace, next_temp).ok()
            }
            Err(_) => None,
        };
        Self {
            shared,
            can_share_by_link: AtomicBool::new(true),
        }
    }

    /// `Some` only while sharing is both enabled and materialized.
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

/// Capability-degrade set: `EMLINK`/`EOPNOTSUPP` mean the upper cannot hard-link a whiteout.
fn is_capability_degraded(err: &Error) -> bool {
    matches!(err.error(), Errno::EMLINK | Errno::EOPNOTSUPP)
}

/// Invariants: `workdir_name` is non-empty and unique among live temps; one owner at a time.
#[derive(Debug)]
struct WhiteoutHandle {
    workdir_name: String,
    dentry: Arc<Dentry>,
}

/// The physical whiteout forms, an enum because mknod and create+xattr behave differently.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WhiteoutRepresentation {
    CharDevice,
    Xattr,
}

/// Chooses the physical whiteout form for the published capability; only `Unsupported` fails.
fn whiteout_representation(capability: WhiteoutCapability) -> Result<WhiteoutRepresentation> {
    match capability {
        WhiteoutCapability::CharDevice => Ok(WhiteoutRepresentation::CharDevice),
        WhiteoutCapability::Xattr => Ok(WhiteoutRepresentation::Xattr),
        WhiteoutCapability::Unsupported => Err(Error::with_message(
            Errno::EOPNOTSUPP,
            "the upper filesystem supports no whiteout form",
        )),
    }
}

fn create_whiteout_handle(
    workspace: &Arc<Dentry>,
    representation: WhiteoutRepresentation,
    namespace: XattrNamespace,
    next_temp: &AtomicU64,
) -> Result<WhiteoutHandle> {
    match representation {
        WhiteoutRepresentation::CharDevice => {
            let temp = WorkdirTemp::create_from_char_device(
                workspace,
                WHITEOUT_TEMP_NAME_COMPONENT,
                WHITEOUT_CHAR_DEV,
                next_temp,
            )?;
            let (workdir_name, dentry) = temp.into_parts();
            Ok(WhiteoutHandle {
                workdir_name,
                dentry,
            })
        }
        WhiteoutRepresentation::Xattr => {
            // No xattr capability gate is needed: this arm is only reached under `Xattr`.
            let temp = WorkdirTemp::create_from_op(
                workspace,
                WHITEOUT_TEMP_NAME_COMPONENT,
                &CreateOp::general(InodeType::File, InodeMode::empty())?,
                next_temp,
            )?;
            let mut marker_reader = VmReader::from(MARKER_VALUE).to_fallible();
            if let Err(err) = OverlayInode::set_overlay_xattr(
                temp.inode(),
                temp.dentry(),
                OverlayRecordName::Whiteout,
                namespace,
                &mut marker_reader,
                XattrSetFlags::CREATE_OR_REPLACE,
            ) {
                let _ = workspace
                    .as_dir_dentry_or_err()
                    .and_then(|dir| dir.unlink(temp.name()));
                return Err(err);
            }
            let (workdir_name, dentry) = temp.into_parts();
            Ok(WhiteoutHandle {
                workdir_name,
                dentry,
            })
        }
    }
}

impl OverlayFs {
    fn create_whiteout_temp(&self) -> Result<WhiteoutHandle> {
        // `Unsupported` is a hard error: a new whiteout cannot be represented.
        let representation = whiteout_representation(self.policy().whiteout_capability())?;
        let workspace = self.workdir_root_dentry()?;
        let next_temp = self.workdir_temp_counter()?;
        create_whiteout_handle(
            &workspace,
            representation,
            self.policy().xattr_namespace(),
            next_temp,
        )
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
                    if is_capability_degraded(&err) {
                        self.whiteout_cache().latch_degraded();
                    }
                    // Falls back to the one-shot rename, which still hides the name.
                }
            }
        }

        // One-shot temp: create, use, never cache.
        let handle = self.create_whiteout_temp()?;
        let workspace = self.workdir_root_dentry()?;
        let workdir_dir = workspace.as_dir_dentry_or_err()?;
        let upper_dir = upper_parent.as_dir_dentry_or_err()?;
        match replace_target {
            Some(target_type) if target_type.is_directory() => {
                workdir_dir.rename(&handle.workdir_name, &upper_dir, name, RenameMode::Exchange)?;
                if let Err(err) = workdir_dir.rmdir(&handle.workdir_name) {
                    warn!(
                        "overlay whiteout publish: workdir cleanup of the displaced directory \
                         {:?} failed (residue, never a visible source): {:?}",
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
}

/// Non-atomic and pre-commit: a failure aborts the caller and a retry converges; never recurses.
pub(super) fn cleanup_upper_whiteouts(
    upper_dir: &Arc<Dentry>,
    namespace: XattrNamespace,
) -> Result<()> {
    let names = crate::fs::fs_impls::overlayfs::read_child_names(upper_dir.inode())?;
    validate_whiteout_children(upper_dir, &names, namespace)?;
    unlink_rechecked_whiteouts(upper_dir, &names, namespace)?;
    Ok(())
}

/// Each child is re-classified just before unlink, so a swapped entry is refused, not deleted.
fn unlink_rechecked_whiteouts(
    upper_dir: &Arc<Dentry>,
    names: &[String],
    namespace: XattrNamespace,
) -> Result<()> {
    let dir = upper_dir.as_dir_dentry_or_err()?;
    for name in names {
        if !is_whiteout_child(upper_dir, name, namespace)? {
            return Err(Error::with_message(
                Errno::ENOTEMPTY,
                "a hidden non-whiteout entry prevents the overlay directory removal",
            ));
        }
        dir.unlink(name)?;
    }
    Ok(())
}

fn is_whiteout_child(
    upper_dir: &Arc<Dentry>,
    name: &str,
    namespace: XattrNamespace,
) -> Result<bool> {
    let child = upper_dir.as_dir_dentry_or_err()?.lookup_child(name)?;
    is_whiteout_inode(child.inode(), namespace)
}

fn validate_whiteout_children(
    upper_dir: &Arc<Dentry>,
    names: &[String],
    namespace: XattrNamespace,
) -> Result<()> {
    for name in names {
        if !is_whiteout_child(upper_dir, name, namespace)? {
            return Err(Error::with_message(
                Errno::ENOTEMPTY,
                "a hidden non-whiteout entry prevents the overlay directory removal",
            ));
        }
    }
    Ok(())
}
