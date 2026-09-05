// SPDX-License-Identifier: MPL-2.0

//! Mount construction: the one-shot preparation of an overlay filesystem's
//! published state.
//!
//! Construction runs once per mount, before the overlay filesystem object
//! is published: options are parsed, the original roots are resolved and
//! validated, private layer views are built, the runtime workdir is re-homed
//! onto the upper layer view mount, the upper/workdir pair is claimed, the
//! staging workspace is prepared, upper-filesystem capabilities are probed,
//! and the mount policy is assembled from the results.
//!
//! ## Structure
//!
//! | Submodule | Responsibility |
//! | --- | --- |
//! | `options` | parse and validate the mount option string into construction input |
//! | `layer_parts` | resolve original roots, validate them, and build the layer stack |
//! | `inuse` | claim the upper/workdir pair and carry the overlay identity |
//! | `capabilities` | probe upper-filesystem capabilities after the claim |

#![short_vis_path::add(overlayfs)]

pub(super) mod capabilities;
pub(in overlayfs) mod inuse;
mod layer_parts;
pub(super) mod options;

/// Re-exported so mutation paths can classify the mount's whiteout capability.
pub(in overlayfs) use self::capabilities::WhiteoutCapability;
use self::{
    capabilities::UpperFilesystemCapabilities,
    inuse::{UpperWorkdirInuse, Uuid},
    options::MountOptions,
};
use super::{
    OverlayFs,
    policy::{MountPolicy, UuidMode, XinoMode},
};
use crate::{
    fs::{
        fs_impls::overlayfs::inode::{IdentityPolicy, InodeCache, WhiteoutCache},
        pseudofs::AnonDeviceId,
        vfs::{
            file_system::{FsEventSubscriberStats, FsFlags},
            path::{Path, PerMountFlags},
            registry::FsCreationCtx,
            xattr::XattrNamespace,
        },
    },
    prelude::*,
};

impl OverlayFs {
    pub(in overlayfs) fn new(fs_creation_ctx: &FsCreationCtx) -> Result<Arc<Self>> {
        let options = MountOptions::parse(fs_creation_ctx.args(), fs_creation_ctx.flags())?;

        // Resolve original roots in option order: upperdir -> lowerdir -> workdir.
        let (upper_workdir_roots, lowers_original) = match &options.upper_workdir {
            Some(pair) => {
                let upper_original = layer_parts::resolve_root_dir(
                    &pair.upper_dir,
                    "the layer root is not a directory",
                )?;
                let lowers_original = layer_parts::resolve_lower_roots(&options.lower_dirs)?;
                let workdir_original =
                    layer_parts::resolve_root_dir(&pair.workdir, "workdir is not a directory")?;
                (
                    Some((
                        (pair.upper_dir.as_str(), pair.workdir.as_str()),
                        (upper_original, workdir_original),
                    )),
                    lowers_original,
                )
            }
            None => (None, layer_parts::resolve_lower_roots(&options.lower_dirs)?),
        };

        let upper_on_read_only_mount =
            upper_workdir_roots
                .as_ref()
                .is_some_and(|(_, (upper_original, _))| {
                    upper_original
                        .mount_node()
                        .flags()
                        .contains(PerMountFlags::RDONLY)
                });
        let is_effective_read_only = match upper_workdir_roots.as_ref() {
            Some((_, (upper_original, _))) => {
                options.is_forced_read_only
                    || upper_original
                        .mount_node()
                        .fs()
                        .flags()
                        .contains(FsFlags::RDONLY)
                    || upper_original
                        .mount_node()
                        .flags()
                        .contains(PerMountFlags::RDONLY)
            }
            None => true,
        };
        if options.uuid_mode == Some(UuidMode::On) && is_effective_read_only {
            info!(
                "option `uuid=on` is ineffective on a read-only overlay; the overlay uuid is not persisted"
            );
        }
        if upper_on_read_only_mount {
            warn!(
                "the overlay upperdir is on a read-only mount; the overlay is treated as read-only \
                 (`uuid=on` is ineffective, no workdir is prepared, and upper capabilities are not probed)"
            );
        }

        let xino_mode = options.xino_mode.unwrap_or(XinoMode::Auto);
        let xattr_namespace = if options.is_userxattr {
            XattrNamespace::User
        } else {
            XattrNamespace::Trusted
        };

        let mut upper_workdir_pair = None;
        let mut upper_capabilities = None;
        let mut uuid = None;
        let layer_stack;
        let whiteout_cache;

        if let Some(((upper_dir, work_dir), (upper_original, workdir_original))) =
            &upper_workdir_roots
        {
            // A read-only upper cannot back a writable overlay if the mount stays writable.
            if !options.is_forced_read_only
                && upper_original
                    .mount_node()
                    .fs()
                    .flags()
                    .contains(FsFlags::RDONLY)
            {
                return_errno_with_message!(Errno::EROFS, "the upper filesystem is read-only");
            }

            // An overlay cannot back another's upper: writes re-enter the backing overlay.
            let upper_mount_fs = upper_original.mount_node().fs().clone();
            if Arc::downcast::<OverlayFs>(upper_mount_fs).is_ok() {
                return Err(Error::with_message(
                    Errno::EINVAL,
                    "the overlay upperdir must not be on an overlayfs",
                ));
            }

            UpperWorkdirInuse::validate_pair(upper_original, workdir_original)?;
            layer_parts::validate_layer_overlap(Some(upper_original), &lowers_original)?;
            layer_parts::validate_workdir_against_lowers(workdir_original, &lowers_original)?;
            layer_parts::verify_inode_instance_stability(upper_dir, upper_original.inode())?;
            layer_parts::verify_inode_instance_stability(work_dir, workdir_original.inode())?;

            let uuid_mode = options.uuid_mode.unwrap_or(UuidMode::Auto);
            let identity = if is_effective_read_only {
                Ok(Uuid::generate())
            } else {
                UpperWorkdirInuse::determine_identity(
                    upper_original.inode(),
                    uuid_mode,
                    xattr_namespace,
                )
            }?;

            layer_stack = layer_parts::build_layer_stack(Some(upper_original), &lowers_original)?;
            let upper_layer = layer_stack.upper_layer()?;
            let upper_runtime =
                Path::new(upper_layer.mount.clone(), upper_layer.root_dentry().clone());
            let workdir_runtime =
                Path::new(upper_layer.mount.clone(), workdir_original.dentry().clone());

            let mut claimed_pair =
                UpperWorkdirInuse::claim(&upper_runtime, &workdir_runtime, identity)?;

            if !is_effective_read_only {
                claimed_pair.prepare_workdir(&workdir_runtime)?;

                let capabilities =
                    UpperFilesystemCapabilities::probe(&claimed_pair, xattr_namespace)?;
                capabilities.validate_required_capabilities()?;
                let is_uuid_effective = capabilities.validate_uuid_support(uuid_mode)?;

                if is_uuid_effective {
                    match claimed_pair.persist_identity(xattr_namespace) {
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

                let workspace = claimed_pair.workdir_workspace()?;
                let whiteout_cache_for_mount = WhiteoutCache::with_shared_whiteout(
                    capabilities.whiteout_capability(),
                    workspace,
                    xattr_namespace,
                    claimed_pair.next_temp_counter(),
                );
                upper_capabilities = Some(capabilities);
                upper_workdir_pair = Some(claimed_pair);
                whiteout_cache = whiteout_cache_for_mount;
            } else {
                upper_workdir_pair = Some(claimed_pair);
                whiteout_cache = WhiteoutCache::new();
            }
        } else {
            layer_parts::validate_layer_overlap(None, &lowers_original)?;
            layer_stack = layer_parts::build_layer_stack(None, &lowers_original)?;
            whiteout_cache = WhiteoutCache::new();
        }

        let policy = MountPolicy::assemble(
            is_effective_read_only,
            options.is_default_permissions,
            xattr_namespace,
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

        let is_all_layers_same_fs = layer_stack.is_all_layers_same_fs();

        let identity = IdentityPolicy::new(
            overlay_dev_id,
            IdentityPolicy::XINO_SHIFT,
            xino_mode,
            is_all_layers_same_fs,
        )?;

        let inodes = InodeCache::new();

        let overlay_fs = Arc::new_cyclic(move |weak| OverlayFs {
            layer_stack,
            policy,
            identity,
            upper_workdir_pair,
            whiteout_cache,
            inodes,
            fs_event_stats: FsEventSubscriberStats::new(),
            _anon_device_id: anon_device_id,
            self_weak: weak.clone(),
        });
        Ok(overlay_fs)
    }
}
