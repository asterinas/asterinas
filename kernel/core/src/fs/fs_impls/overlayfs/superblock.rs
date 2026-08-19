// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(overlayfs)]
//! The overlay filesystem object and its VFS-facing superblock surface.
//!
//! `OverlayFs` is the per-mount overlay filesystem object that owns the
//! layer stack, claims, policy, and projection state; the `FileSystem`
//! impl forwards the superblock surface to the underlying real filesystem.

use crate::{
    fs::{
        fs_impls::overlayfs::{
            dir::whiteout::WhiteoutCache,
            fs_type::OVERLAY_FS_NAME,
            inode::OverlayInode,
            metadata_security::xattr::XattrPolicy,
            mount::{claims::UpperWorkdirClaim, layers::LayerStack, policy::MountPolicy},
            projection::{BindingCache, IdentityPolicy, InodeCache, RealObject, RealObjectKey},
        },
        pseudofs::AnonDeviceId,
        vfs::{
            file_system::{FileSystem, FsEventSubscriberStats, FsFlags, SuperBlock},
            inode::Inode,
        },
    },
    prelude::*,
};

/// The top-level overlay filesystem object.
///
/// Created by [`OverlayFs::new`]; owns the layer stack, claims, policy,
/// projection state.
pub(super) struct OverlayFs {
    pub(super) layer_stack: LayerStack,
    /// The claimed upper/workdir pair; `Some` only for writable mounts.
    ///
    /// Established single-threaded before publication,
    /// and released by the claim guard's `Drop`.
    /// The claim additionally pins the prepared workdir staging workspace inode
    /// (`<workdir>/work`) once `prepare_workdir` completes.
    pub(super) claims: Option<UpperWorkdirClaim>,
    pub(super) policy: MountPolicy,
    pub(super) fs_event_stats: FsEventSubscriberStats,
    pub(super) self_weak: Weak<OverlayFs>,
    /// The mount-wide binding cache — the first source for `(parent, name)`
    /// lookup results.
    ///
    /// A positive binding pins its inode, a negative one pins its barrier;
    /// insert/update happen under the caller's parent directory transaction
    /// lock.
    pub(super) bindings: BindingCache,
    /// The mount-wide inode identity-reuse cache.
    ///
    /// Maps each `RealObjectKey` to a `Weak<OverlayInode>`.
    pub(super) inodes: InodeCache,
    /// The dev/ino projection policy.
    pub(super) identity: IdentityPolicy,
    /// The overlay `AnonDeviceId` RAII guard, retained for the mount lifetime.
    ///
    /// `IdentityPolicy::overlay_dev_id` copies the device id, so the guard
    /// must live on the published `OverlayFs` (the substrate-idiomatic owner —
    /// every Asterinas pseudo-fs and the legacy overlayfs hold `AnonDeviceId`
    /// on the fs struct) or the minor number could be recycled under a live
    /// mount. The `_`-prefixed name mirrors the sibling pseudo-fs precedent
    /// and suppresses the unused-field lint.
    pub(super) _anon_device_id: AnonDeviceId,
    /// The xattr classification policy.
    pub(super) xattr_policy: XattrPolicy,
    /// The mount-scoped reusable whiteout cache.
    ///
    /// Bounded to one workdir staging slot.
    pub(super) whiteout_cache: Mutex<WhiteoutCache>,
}

impl OverlayFs {
    /// Returns the inode-cache key of the overlay mount root.
    pub(super) fn root_visible_key(&self) -> RealObjectKey {
        match self.layer_stack.upper_layer() {
            Ok(upper) => RealObjectKey::from_source(&RealObject::identity_only(
                0,
                upper.root_path.inode().clone(),
                upper.fsid,
                upper.container_dev_id,
            )),
            Err(_) => {
                let top = self
                    .layer_stack
                    .lower_layers()
                    .first()
                    .expect("the layer stack always carries at least one lower layer");
                RealObjectKey::from_source(&RealObject::identity_only(
                    1,
                    top.root_path.inode().clone(),
                    top.fsid,
                    top.container_dev_id,
                ))
            }
        }
    }

    /// Returns the real filesystem that superblock hooks forward to: the upper
    /// filesystem for writable mounts, otherwise the topmost lower layer.
    ///
    /// `sync`/`statfs` semantics are forwarded to this filesystem.
    fn selected_real_fs(&self) -> &Arc<dyn FileSystem> {
        self.layer_stack
            .upper_layer()
            .ok()
            .map_or(&self.layer_stack.lower_layers()[0].fs, |upper| &upper.fs)
    }
}

impl FileSystem for OverlayFs {
    fn name(&self) -> &'static str {
        OVERLAY_FS_NAME
    }

    fn sync(&self) -> Result<()> {
        self.selected_real_fs().sync()
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        OverlayInode::new_root(self.self_weak.clone())
    }

    fn sb(&self) -> SuperBlock {
        let mut super_block = self.selected_real_fs().sb();
        if let Some(uuid) = self.policy.uuid() {
            super_block.fsid = uuid.value();
        }
        super_block
    }

    fn flags(&self) -> FsFlags {
        if self.policy.is_effective_read_only() {
            FsFlags::RDONLY
        } else {
            FsFlags::empty()
        }
    }

    fn set_fs_flags(&self, flags: FsFlags, _data: Option<&str>, _ctx: &Context) -> Result<()> {
        let current_flags = self.flags();
        if current_flags.contains(FsFlags::RDONLY) && !flags.contains(FsFlags::RDONLY) {
            return Err(Error::new(Errno::EROFS));
        }
        if flags != current_flags {
            return Err(Error::with_message(
                Errno::EINVAL,
                "unsupported overlayfs remount delta",
            ));
        }
        Ok(())
    }

    fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats {
        &self.fs_event_stats
    }
}
