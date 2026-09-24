// SPDX-License-Identifier: MPL-2.0

//! Mount construction: the one-shot preparation of an overlay filesystem's
//! published state.
//!
//! Construction runs once per mount, before the overlay filesystem object
//! is published: options are parsed, the original roots are resolved and
//! validated, the upper/workdir pair is claimed, the layer stack is built,
//! the staging workspace is prepared, upper-filesystem capabilities are
//! probed, and the mount policy is assembled from the results.
//!
//! # Module map
//!
//! | Submodule | Responsibility |
//! |---|---|
//! | [`options`] | the mount option parse and validation |
//! | [`layer_parts`] | the root resolution, the overlap checks, and the layer-stack build |
//! | [`inuse`] | the exclusive upper/workdir claim and the overlay uuid |
//! | [`capabilities`] | the upper-filesystem capability probes |
//! | [`policy`] | the per-mount decisions settled after the probes, published as fixed state |

#![short_vis_path::add(overlayfs)]

mod capabilities;
pub(in overlayfs) mod inuse;
mod layer_parts;
mod options;
pub(in overlayfs) mod policy;

// Re-exported so mutation paths can classify the mount's whiteout capability.
pub(in overlayfs) use self::capabilities::WhiteoutCapability;
// Re-exported so the identity policy can read the `xino` mode the mount parsed.
pub(in overlayfs) use self::options::XinoMode;
use self::{
    capabilities::UpperFilesystemCapabilities,
    layer_parts::MountRoots,
    options::{MountOptions, UuidMode},
    policy::MountPolicy,
};
use super::OverlayFs;
use crate::{
    fs::{
        fs_impls::overlayfs::inode::{InodeCache, WhiteoutCache},
        pseudofs::AnonDeviceId,
        vfs::{file_system::FsEventSubscriberStats, registry::FsCreationCtx},
    },
    prelude::*,
};

impl OverlayFs {
    pub(in overlayfs) fn new(fs_creation_ctx: &FsCreationCtx) -> Result<Arc<Self>> {
        let options = MountOptions::parse(fs_creation_ctx.args(), fs_creation_ctx.flags())?;
        let ctx = fs_creation_ctx.ctx().ok_or_else(|| {
            Error::with_message(Errno::EINVAL, "the overlay mount has no task context")
        })?;
        let roots = MountRoots::resolve(ctx, &options)?;
        let is_effective_read_only = roots.is_effective_read_only(&options);
        let upper_workdir = roots.claim_upper_workdir(&options)?;
        let layer_stack = roots.into_layer_stack()?;

        let xino_mode = options.xino_mode.unwrap_or(XinoMode::Auto);
        let xattr_namespace = options.xattr_namespace;

        let mut upper_workdir_inuse = None;
        let mut upper_capabilities = None;
        let mut uuid = None;
        let whiteout_cache;

        if let Some(mut inuse_pair) = upper_workdir {
            let uuid_mode = options.uuid_mode.unwrap_or(UuidMode::Auto);
            if !is_effective_read_only {
                inuse_pair.prepare_workdir()?;

                let capabilities =
                    UpperFilesystemCapabilities::probe(&inuse_pair, xattr_namespace)?;
                // Whiteout suppression and the merge trust the reported type; no stat fallback.
                if !capabilities.can_report_directory_type() {
                    return_errno_with_message!(
                        Errno::EOPNOTSUPP,
                        "the upper filesystem cannot report directory entry types"
                    );
                }
                let is_uuid_effective = capabilities.validate_uuid_support(uuid_mode)?;

                if is_uuid_effective {
                    match inuse_pair.persist_identity(xattr_namespace) {
                        Ok(persisted) => {
                            uuid = Some(persisted);
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

                let whiteout_cache_for_mount = WhiteoutCache::with_shared_whiteout(
                    capabilities.whiteout_capability(),
                    &inuse_pair,
                    xattr_namespace,
                );
                upper_capabilities = Some(capabilities);
                upper_workdir_inuse = Some(inuse_pair);
                whiteout_cache = whiteout_cache_for_mount;
            } else {
                upper_workdir_inuse = Some(inuse_pair);
                whiteout_cache = WhiteoutCache::new();
            }
        } else {
            whiteout_cache = WhiteoutCache::new();
        }

        let policy = MountPolicy::assemble(
            is_effective_read_only,
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

        let identity = OverlayFs::new_identity_policy(
            anon_device_id,
            &layer_stack,
            xino_mode,
            is_effective_read_only,
            &policy,
        );

        let inodes = InodeCache::new();

        let overlay_fs = Arc::new_cyclic(move |weak| OverlayFs {
            layer_stack,
            policy,
            identity,
            upper_workdir_inuse,
            whiteout_cache,
            inodes,
            fs_event_stats: FsEventSubscriberStats::new(),
            self_weak: weak.clone(),
        });
        Ok(overlay_fs)
    }
}
