// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(overlayfs)]
//! The overlay filesystem object and its VFS-facing superblock surface.
//!
//! `OverlayFs` is the per-mount overlay filesystem object that owns the
//! layer stack, the upper/workdir in-use claims, the mount policy, and the
//! dev/ino identity translation — the mapping that presents overlay-visible
//! device and inode numbers to the VFS. The `FileSystem` impl forwards the
//! superblock surface to the underlying real filesystem.

pub(super) mod mount;
pub(super) mod policy;

use self::{mount::inuse::UpperWorkdirInuse, policy::MountPolicy};
use crate::{
    fs::{
        fs_impls::overlayfs::{
            fs_type::OVERLAY_FS_NAME,
            inode::{IdentityPolicy, InodeCache, OverlayInode, WhiteoutCache},
            layer::{Layer, LayerStack},
            real::{RealObject, RealObjectKey},
        },
        pseudofs::AnonDeviceId,
        vfs::{
            file_system::{FileSystem, FsEventSubscriberStats, FsFlags, SuperBlock},
            inode::Inode,
            path::Path,
        },
    },
    prelude::*,
};

pub(super) struct OverlayFs {
    layer_stack: LayerStack,
    policy: MountPolicy,
    identity: IdentityPolicy,
    upper_workdir_pair: Option<UpperWorkdirInuse>,
    whiteout_cache: Mutex<WhiteoutCache>,
    inodes: InodeCache,
    fs_event_stats: FsEventSubscriberStats,
    /// Keeps the mount's `AnonDeviceId` alive: `identity` copies its device
    /// id, so an early drop could let the minor be recycled under a live mount.
    _anon_device_id: AnonDeviceId,
    self_weak: Weak<OverlayFs>,
}

impl OverlayFs {
    pub(super) fn layer_stack(&self) -> &LayerStack {
        &self.layer_stack
    }

    pub(super) fn layer(&self, layer_index: usize) -> &Layer {
        if layer_index == 0 {
            return self
                .layer_stack
                .upper
                .as_ref()
                .expect("a real object with layer index 0 references the upper layer");
        }
        self.layer_stack
            .lowers
            .get(layer_index - 1)
            .expect("a real object references a configured lower layer")
    }

    pub(super) fn real_object_path(&self, real: &RealObject) -> Path {
        Path::new(
            self.layer(real.layer_index()).mount.clone(),
            real.dentry().clone(),
        )
    }

    pub(super) fn real_object_key(&self, real: &RealObject) -> RealObjectKey {
        RealObjectKey::from_source(self.layer(real.layer_index()).fsid, real)
    }

    pub(super) fn upper_workdir_pair(&self) -> &Option<UpperWorkdirInuse> {
        &self.upper_workdir_pair
    }

    pub(super) fn policy(&self) -> &MountPolicy {
        &self.policy
    }

    pub(super) fn self_weak(&self) -> &Weak<OverlayFs> {
        &self.self_weak
    }

    pub(super) fn inodes(&self) -> &InodeCache {
        &self.inodes
    }

    pub(super) fn identity(&self) -> &IdentityPolicy {
        &self.identity
    }

    pub(super) fn whiteout_cache(&self) -> &Mutex<WhiteoutCache> {
        &self.whiteout_cache
    }

    fn selected_real_fs(&self) -> &Arc<dyn FileSystem> {
        self.layer_stack
            .upper_layer()
            .ok()
            .map_or(self.layer_stack.lower_layers()[0].mount.fs(), |upper| {
                upper.mount.fs()
            })
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
