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
            fs::OverlayFs,
            inode::{
                OverlayInode,
                copyup::workdir::WorkdirTempRequest,
                is_whiteout_inode,
                xattr::{OverlayRecordName, OverlayXattrPrefix, WHITEOUT_MARKER_VALUE},
            },
        },
        vfs::{
            inode::{MknodType, RenameMode},
            path::Path,
            xattr::XattrSetFlags,
        },
    },
    prelude::*,
};

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

    fn store(&mut self, handle: WhiteoutHandle) {
        if self.cached.replace(handle).is_some() {
            warn!(
                "overlay whiteout cache slot occupied at store; the stale cached whiteout is \
                 dropped (workdir-cleanup residue, never a visible source)"
            );
        }
    }

    fn disable_sharing(&mut self) {
        self.can_share_by_link = false;
    }
}

/// Invariants: `workdir_name` is non-empty and unique among live temps; the
/// handle is owned by the cache slot or a single mutation.
#[derive(Debug)]
struct WhiteoutHandle {
    workdir_name: String,
    path: Path,
}

/// The physical whiteout forms, classified as an enum rather than a bool
/// because the two forms carry different recipe behavior (mknod vs
/// create+xattr).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WhiteoutRepresentation {
    CharDevice,
    Xattr,
}

impl OverlayFs {
    fn whiteout_representation(&self) -> Result<WhiteoutRepresentation> {
        let capabilities = self.policy().upper_capabilities().ok_or_else(|| {
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
                let temp = self.create_workdir_temp(
                    WHITEOUT_TEMP_NAME_COMPONENT,
                    WorkdirTempRequest::Create {
                        kind: InodeType::File,
                        mode: InodeMode::empty(),
                    },
                )?;
                let mut marker_reader = VmReader::from(WHITEOUT_MARKER_VALUE).to_fallible();
                if let Err(err) = OverlayInode::set_overlay_xattr(
                    temp.inode(),
                    temp.dentry(),
                    OverlayRecordName::Whiteout,
                    self.policy().xattr_prefix(),
                    &mut marker_reader,
                    XattrSetFlags::CREATE_OR_REPLACE,
                ) {
                    let _ = self.cleanup_workdir_temp(temp.name(), temp.kind());
                    return Err(err);
                }
                temp.into_parts()
            }
        };
        Ok(WhiteoutHandle { workdir_name, path })
    }

    pub(super) fn publish_whiteout(
        &self,
        upper_parent_path: &Path,
        name: &str,
        replace_target: Option<InodeType>,
    ) -> Result<()> {
        let (cached, can_share_by_link) = {
            let mut cache = self.whiteout_cache().lock();
            let cached = cache.take();
            let can_share_by_link = cache.can_share_by_link;
            (cached, can_share_by_link)
        };

        let handle = match cached {
            Some(handle) => handle,
            None => self.create_whiteout_temp()?,
        };

        let workdir_path = self.workdir_root_path()?;
        // Publishing a whiteout makes the parent impure, so the marker is
        // set before the physical publish. The marker is a best-effort
        // cache hint, so a marker failure must not abort the publish.
        if let Err(err) = OverlayInode::set_impure_marker(
            upper_parent_path.inode(),
            upper_parent_path.dentry(),
            self.policy().xattr_prefix(),
        ) {
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
                            self.whiteout_cache().lock().store(handle);
                            return Ok(());
                        }
                        Err(err) if matches!(err.error(), Errno::EMLINK | Errno::EOPNOTSUPP) => {
                            self.whiteout_cache().lock().disable_sharing();
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

/// Non-atomic and pre-commit: a failure aborts the caller and a retry
/// converges; never recurses into directories.
pub(super) fn cleanup_upper_whiteouts(
    upper_dir_path: &Path,
    prefix: OverlayXattrPrefix,
) -> Result<()> {
    let names = crate::fs::fs_impls::overlayfs::read_child_names(upper_dir_path.inode())?;
    validate_whiteout_children(upper_dir_path, &names, prefix)?;
    unlink_rechecked_whiteouts(upper_dir_path, &names, prefix)?;
    Ok(())
}

/// Each child is re-classified immediately before its `unlink`, so an
/// entry swapped in since the validation pass is refused (`ENOTEMPTY`)
/// instead of deleted; the check-to-use window narrows but cannot
/// close, so the upper directory must not be modified concurrently.
fn unlink_rechecked_whiteouts(
    upper_dir_path: &Path,
    names: &[String],
    prefix: OverlayXattrPrefix,
) -> Result<()> {
    for name in names {
        if !is_whiteout_child(upper_dir_path, name, prefix)? {
            return Err(Error::with_message(
                Errno::ENOTEMPTY,
                "a hidden non-whiteout entry prevents the overlay directory removal",
            ));
        }
        upper_dir_path.unlink(name)?;
    }
    Ok(())
}

fn is_whiteout_child(
    upper_dir_path: &Path,
    name: &str,
    prefix: OverlayXattrPrefix,
) -> Result<bool> {
    let child_path = super::super::super::lookup_child_path(upper_dir_path, name)?;
    is_whiteout_inode(child_path.inode(), prefix)
}

fn validate_whiteout_children(
    upper_dir_path: &Path,
    names: &[String],
    prefix: OverlayXattrPrefix,
) -> Result<()> {
    for name in names {
        if !is_whiteout_child(upper_dir_path, name, prefix)? {
            return Err(Error::with_message(
                Errno::ENOTEMPTY,
                "a hidden non-whiteout entry prevents the overlay directory removal",
            ));
        }
    }
    Ok(())
}
