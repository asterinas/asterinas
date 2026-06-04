// SPDX-License-Identifier: MPL-2.0

//! VFS filesystem-type registration for ext2.
//!
//! `Ext2Type` implements the `FsType` trait so the VFS layer can
//! discover and mount ext2 volumes by name (`"ext2"`).

use aster_systree::SysNode;
use device_id::DeviceId;

use super::{fs::Ext2, prelude::*};
use crate::fs::vfs::{
    file_system::FileSystem,
    registry::{FsCache, FsCreationCtx, FsProperties, FsType},
};

/// VFS-visible Ext2 filesystem type.
pub(super) struct Ext2Type {
    cache: FsCache<DeviceId>,
}

/// The singleton registered with the VFS registry. Holds the per-fs-type
/// `(FileSystem, root Dentry)` cache so two mounts of the same block device
/// share one ext2 superblock and one root dentry, matching Linux's
/// `sget_fc`-based behavior.
pub(super) static EXT2_TYPE: Ext2Type = Ext2Type {
    cache: FsCache::new(),
};

impl FsType for Ext2Type {
    type Key = DeviceId;

    fn name(&self) -> &'static str {
        "ext2"
    }

    fn properties(&self) -> FsProperties {
        FsProperties::NEED_DISK
    }

    fn create(&self, fs_creation_ctx: &FsCreationCtx) -> Result<Arc<dyn FileSystem>> {
        let disk = fs_creation_ctx.resolve_block_device()?;
        let args = fs_creation_ctx.args();
        Ext2::open(disk, args).map(|fs| fs as Arc<dyn FileSystem>)
    }

    fn obtain_key(&self, fs_creation_ctx: &FsCreationCtx) -> Option<DeviceId> {
        fs_creation_ctx
            .resolve_block_device()
            .ok()
            .map(|disk| disk.id())
    }

    fn cache(&self) -> Option<&FsCache<DeviceId>> {
        Some(&self.cache)
    }

    fn sysnode(&self) -> Option<Arc<dyn SysNode>> {
        None
    }
}
