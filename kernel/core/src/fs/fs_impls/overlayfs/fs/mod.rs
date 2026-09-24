// SPDX-License-Identifier: MPL-2.0

//! The overlay filesystem object and its VFS-facing superblock surface.
//!
//! [`OverlayFs`] is the per-mount overlay filesystem object. Its [`FileSystem`]
//! impl forwards the superblock surface to the underlying real filesystem.
//!
//! The `overlay` registration and the mount requests answered under it live in
//! [`fs_type`].

mod fs_type;
pub(super) mod mount;

pub(in crate::fs::fs_impls) fn init() {
    crate::fs::vfs::registry::register(&fs_type::OverlayFsType).unwrap();
}

use self::mount::{inuse::UpperWorkdirInuse, policy::MountPolicy};
use crate::{
    fs::{
        fs_impls::overlayfs::{
            fs::fs_type::OVERLAY_FS_NAME,
            inode::{IdentityPolicy, InodeCache, WhiteoutCache},
            layer::LayerStack,
            real::{RealObject, RealObjectStack},
        },
        vfs::{
            file_system::{FileSystem, FsEventSubscriberStats, FsFlags, SuperBlock},
            inode::Inode,
        },
    },
    prelude::*,
};

/// Holds one mount's overlay state: what construction settled and what every VFS entry reads.
pub(super) struct OverlayFs {
    layer_stack: LayerStack,
    policy: MountPolicy,
    identity: IdentityPolicy,
    upper_workdir_inuse: Option<UpperWorkdirInuse>,
    whiteout_cache: WhiteoutCache,
    inodes: InodeCache,
    fs_event_stats: FsEventSubscriberStats,
    self_weak: Weak<OverlayFs>,
}

impl OverlayFs {
    /// Returns the upper and workdir this mount holds in use; only a mount given both has one.
    pub(super) fn upper_workdir_inuse(&self) -> &UpperWorkdirInuse {
        self.upper_workdir_inuse
            .as_ref()
            .expect("a writable overlay mount claims the upper and workdir pair")
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

    pub(super) fn whiteout_cache(&self) -> &WhiteoutCache {
        &self.whiteout_cache
    }
}

impl FileSystem for OverlayFs {
    fn name(&self) -> &'static str {
        OVERLAY_FS_NAME
    }

    /// Syncs the upper layer; a mount without an upper has nothing to sync.
    fn sync(&self) -> Result<()> {
        match self.layer_stack.upper_layer().ok() {
            Some(upper) => upper.fs().sync(),
            None => Ok(()),
        }
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        let upper = self
            .layer_stack
            .upper_layer()
            .ok()
            .map(|layer| RealObject::new_upper(layer.root_dentry().clone()));
        let lowers: Vec<_> = self
            .layer_stack
            .lower_layers()
            .iter()
            .enumerate()
            .map(|(layer_index, layer)| {
                RealObject::new_lower(layer_index + 1, layer.root_dentry().clone())
            })
            .collect();
        let facts = RealObjectStack::new(upper, lowers);
        // A mount root is a directory, so its projection reads no hard-link count and cannot fail.
        self.project_inode(facts)
            .expect("the mount root is a directory, so its projection reads no hard-link count")
    }

    fn sb(&self) -> SuperBlock {
        let topmost_layer = self
            .layer_stack
            .upper_layer()
            .ok()
            .unwrap_or(&self.layer_stack.lower_layers()[0]);
        let mut super_block = topmost_layer.fs().sb();
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
