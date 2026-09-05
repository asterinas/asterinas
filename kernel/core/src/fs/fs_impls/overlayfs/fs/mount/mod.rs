// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(overlayfs)]
//! Mount construction: the one-shot preparation of an overlay filesystem's
//! published state.
//!
//! Construction runs once per mount, before the overlay filesystem object
//! is published: options are parsed and validated, the layer stack is
//! assembled, the upper/workdir pair is claimed, upper-filesystem
//! capabilities are probed, and the mount policy is assembled from the
//! results.
//!
//! ## Structure
//!
//! | Submodule | Responsibility |
//! | --- | --- |
//! | `options` | parse and validate the mount option string into construction input |
//! | `layer_parts` | resolve layer roots and assemble the layer stack |
//! | `inuse` | claim the upper/workdir pair and carry the overlay identity |
//! | `capabilities` | probe upper-filesystem capabilities after the claim |

pub(super) mod capabilities;
pub(in overlayfs) mod inuse;
mod layer_parts;
pub(super) mod options;

use self::{
    capabilities::UpperFilesystemCapabilities,
    inuse::{UpperWorkdirInuse, Uuid},
    layer_parts::{resolve_root_path, verify_inode_instance_stability},
    options::MountOptions,
};
use super::{
    OverlayFs,
    policy::{MountPolicy, UuidMode, XinoMode},
};
use crate::{
    fs::{
        fs_impls::overlayfs::{
            inode::{
                IdentityPolicy, InodeCache, OverlayXattrPrefix, WhiteoutCache, collect_layer_devs,
            },
            layer::LayerStack,
        },
        pseudofs::AnonDeviceId,
        vfs::{
            file_system::{FsEventSubscriberStats, FsFlags},
            path::Path,
            registry::FsCreationCtx,
        },
    },
    prelude::*,
};

impl OverlayFs {
    pub(in overlayfs) fn new(fs_creation_ctx: &FsCreationCtx) -> Result<Arc<Self>> {
        let options = MountOptions::parse(fs_creation_ctx.args(), fs_creation_ctx.flags())?;

        let layer_stack = LayerStack::assemble(
            options.upper_dir.clone(),
            options.lower_dirs.clone(),
            options.is_forced_read_only,
        )?;

        let is_effective_read_only = match &layer_stack.upper {
            Some(upper) => {
                options.is_forced_read_only || upper.mount.fs().flags().contains(FsFlags::RDONLY)
            }
            None => true,
        };
        if options.uuid_mode == Some(UuidMode::On) && is_effective_read_only {
            info!(
                "option `uuid=on` is ineffective on a read-only overlay; the overlay uuid is not persisted"
            );
        }

        let xino_mode = options.xino_mode.unwrap_or(XinoMode::Auto);
        let xattr_prefix = if options.is_userxattr {
            OverlayXattrPrefix::User
        } else {
            OverlayXattrPrefix::Trusted
        };

        let mut upper_workdir_pair = None;
        let mut upper_capabilities = None;
        let mut uuid = None;
        if let Some(upper) = &layer_stack.upper {
            // The parse verify invariant guarantees both option strings exist
            // for an upper-backed overlay; these conversions are defensive.
            let upper_dir = options.upper_dir.as_deref().ok_or_else(|| {
                Error::with_message(Errno::EINVAL, "internal error: missing upperdir option")
            })?;
            let work_dir = options.work_dir.as_deref().ok_or_else(|| {
                Error::with_message(Errno::EINVAL, "internal error: missing workdir option")
            })?;

            // The workdir is not a layer: building it on the upper view's
            // mount keeps every workdir↔upper rename/link on one mount.
            let upper_path = upper.root_path();
            // An overlay view must not serve as another overlay's upper:
            // upper writes would drive the backing overlay's state machine,
            // and the in-use claim would land on the view inode instead of
            // the backing mount's claimed root (upstream 76bc8e2843b6).
            let upper_mount_fs = upper_path.mount_node().fs().clone();
            if Arc::downcast::<OverlayFs>(upper_mount_fs).is_ok() {
                return Err(Error::with_message(
                    Errno::EINVAL,
                    "the overlay upperdir must not be on an overlayfs",
                ));
            }
            let workdir_path = {
                let workdir_dentry = resolve_root_path(work_dir)?.dentry().clone();
                Path::new(upper_path.mount_node().clone(), workdir_dentry)
            };
            UpperWorkdirInuse::validate_pair(&upper_path, &workdir_path)?;
            layer_stack.validate_workdir_against_lowers(&workdir_path)?;
            verify_inode_instance_stability(upper_dir, upper.root_dentry().inode())?;
            verify_inode_instance_stability(work_dir, workdir_path.inode())?;

            let uuid_mode = options.uuid_mode.unwrap_or(UuidMode::Auto);
            let identity = if is_effective_read_only {
                Ok(Uuid::generate())
            } else {
                UpperWorkdirInuse::determine_identity(
                    upper.root_dentry().inode(),
                    uuid_mode,
                    xattr_prefix,
                )
            }?;

            let mut claimed_pair = UpperWorkdirInuse::claim(&upper_path, &workdir_path, identity)?;

            if !is_effective_read_only {
                claimed_pair.prepare_workdir(&workdir_path)?;

                let capabilities = UpperFilesystemCapabilities::probe(
                    upper.root_dentry().inode(),
                    claimed_pair.workdir_workspace_path()?,
                    xattr_prefix,
                )?;
                let is_uuid_effective = capabilities.validate_uuid_support(uuid_mode)?;

                if is_uuid_effective {
                    match claimed_pair.persist_identity(xattr_prefix) {
                        Ok(()) => {
                            uuid = Some(identity);
                        }
                        Err(persist_err) => match uuid_mode {
                            UuidMode::On => {
                                return_errno_with_message!(
                                    Errno::EOPNOTSUPP,
                                    "failed to persist the overlay uuid"
                                );
                            }
                            UuidMode::Auto => {
                                warn!(
                                    "overlay uuid persistence failed; degrading to not-effective: {:?}",
                                    persist_err
                                );
                            }
                            UuidMode::Off | UuidMode::Null => {}
                        },
                    }
                }

                upper_capabilities = Some(capabilities);
            }
            upper_workdir_pair = Some(claimed_pair);
        }

        let policy = MountPolicy::assemble(
            is_effective_read_only,
            options.is_default_permissions,
            xattr_prefix,
            uuid,
            upper_capabilities,
        );

        let anon_device_id = AnonDeviceId::acquire().ok_or_else(|| {
            Error::with_message(
                Errno::ENOSPC,
                "no anonymous device ID is available for the overlay mount",
            )
        })?;
        let overlay_dev_id = anon_device_id.id();

        let (layer_devs, upper_layer_dev_index) = collect_layer_devs(&layer_stack);

        let identity = IdentityPolicy::new(
            overlay_dev_id,
            &layer_devs,
            upper_layer_dev_index,
            IdentityPolicy::XINO_SHIFT,
            xino_mode,
        )?;

        let inodes = InodeCache::new();

        let overlay_fs = Arc::new_cyclic(move |weak| OverlayFs {
            layer_stack,
            policy,
            identity,
            upper_workdir_pair,
            whiteout_cache: Mutex::new(WhiteoutCache::new()),
            inodes,
            fs_event_stats: FsEventSubscriberStats::new(),
            _anon_device_id: anon_device_id,
            self_weak: weak.clone(),
        });
        Ok(overlay_fs)
    }
}
