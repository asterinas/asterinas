// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(overlayfs)]
//! Construction orchestration for the overlay filesystem ([`OverlayFs::new`]).
//!
//! [`OverlayFs::new`] is the single constructor that builds the mount
//! resource/policy state in this order:
//!
//! - Parse options.
//! - Assemble and validate/claim the layer/upper/workdir state.
//! - On writable mounts, prepare the workdir, probe capabilities, and
//!   persist the effective UUID.
//! - Assemble [`MountPolicy`].
//! - Wire projection state and publish the `Arc<OverlayFs>`.
//!
//! Failure releases claimed resources via RAII.
//!
//! # References
//!
//! - <https://elixir.bootlin.com/linux/v7.0/source/fs/overlayfs/super.c#L1545>
//!   (Linux `ovl_fill_super` mount orchestration)
//! - <https://elixir.bootlin.com/linux/v7.0/source/fs/overlayfs/super.c#L667>
//!   (Linux `ovl_make_workdir`)

use super::{
    claims::{UpperWorkdirClaim, Uuid},
    layers::{self, LayerStack},
    options::{MountOptions, UuidMode},
    policy::{MountPolicy, UpperFilesystemCapabilities},
};
use crate::{
    fs::{
        fs_impls::overlayfs::{
            dir::whiteout::WhiteoutCache,
            metadata_security::xattr::XattrPolicy,
            projection::{BindingCache, IdentityPolicy, InodeCache},
            superblock::OverlayFs,
        },
        pseudofs::AnonDeviceId,
        vfs::{
            file_system::{FsEventSubscriberStats, FsFlags},
            registry::FsCreationCtx,
        },
    },
    prelude::*,
};

impl OverlayFs {
    /// Constructs and publishes a fully prepared overlay filesystem.
    pub(in overlayfs) fn new(fs_creation_ctx: &FsCreationCtx) -> Result<Arc<Self>> {
        let options = MountOptions::parse(fs_creation_ctx.args(), fs_creation_ctx.flags())?;

        let layer_stack = LayerStack::assemble(
            options.upper_dir.clone(),
            options.lower_dirs.clone(),
            options.is_forced_read_only,
        )?;

        let is_effective_read_only = match &layer_stack.upper {
            Some(upper) => {
                options.is_forced_read_only || upper.fs.flags().contains(FsFlags::RDONLY)
            }
            None => true,
        };

        let mut claims = None;
        let mut upper_capabilities = None;
        let mut uuid = None;
        if let Some(upper) = &layer_stack.upper {
            // The parse invariant guarantees both option strings are present
            // for an upper-backed overlay; the conversions below are defensive.
            let upper_dir = options.upper_dir.as_deref().ok_or_else(|| {
                Error::with_message(Errno::EINVAL, "internal error: missing upperdir option")
            })?;
            let work_dir = options.work_dir.as_deref().ok_or_else(|| {
                Error::with_message(Errno::EINVAL, "internal error: missing workdir option")
            })?;

            let upper_path = layers::resolve_root_path(upper_dir)?;
            let workdir_path = layers::resolve_root_path(work_dir)?;
            UpperWorkdirClaim::validate_pair(&upper_path, &workdir_path)?;
            // The workdir is not a layer, so `assemble`'s pairwise check
            // cannot cover it.
            layer_stack.validate_workdir_against_lowers(&workdir_path)?;
            layers::verify_inode_instance_stability(upper_dir, upper.root_path.inode())?;
            layers::verify_inode_instance_stability(work_dir, workdir_path.inode())?;

            let uuid_mode = options.uuid_mode.unwrap_or(UuidMode::Auto);
            let identity = if is_effective_read_only {
                Ok(Uuid::generate())
            } else {
                UpperWorkdirClaim::determine_identity(upper.root_path.inode(), uuid_mode)
            }?;

            let mut claimed_pair = UpperWorkdirClaim::claim(
                upper.root_path.inode().clone(),
                workdir_path.inode().clone(),
                identity,
            )?;

            if !is_effective_read_only {
                claimed_pair.prepare_workdir(&workdir_path)?;

                let capabilities = UpperFilesystemCapabilities::probe(
                    upper.root_path.inode(),
                    claimed_pair.workdir_workspace()?,
                )?;
                let is_uuid_effective = capabilities.validate_uuid_support(uuid_mode)?;

                if is_uuid_effective {
                    match claimed_pair.persist_identity() {
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
            claims = Some(claimed_pair);
        }

        let policy =
            MountPolicy::assemble(is_effective_read_only, &options, uuid, upper_capabilities);

        let anon_device_id = AnonDeviceId::acquire().ok_or_else(|| {
            Error::with_message(
                Errno::ENOSPC,
                "no anonymous device ID is available for the overlay mount",
            )
        })?;
        let overlay_dev_id = anon_device_id.id();

        let (layer_devs, upper_layer_dev_index) = layer_stack.collect_layer_devs();

        let identity = IdentityPolicy::new(
            overlay_dev_id,
            &layer_devs,
            upper_layer_dev_index,
            IdentityPolicy::XINO_SHIFT,
            policy.xino_mode(),
        )?;

        let bindings = BindingCache::new();
        let inodes = InodeCache::new();

        let overlay_fs = Arc::new_cyclic(move |weak| OverlayFs {
            layer_stack,
            claims,
            policy,
            fs_event_stats: FsEventSubscriberStats::new(),
            self_weak: weak.clone(),
            bindings,
            inodes,
            identity,
            _anon_device_id: anon_device_id,
            xattr_policy: XattrPolicy,
            whiteout_cache: Mutex::new(WhiteoutCache::new()),
        });
        Ok(overlay_fs)
    }
}
