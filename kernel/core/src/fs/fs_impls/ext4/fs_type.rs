// SPDX-License-Identifier: MPL-2.0

//! VFS filesystem-type registration for ext2.
//!
//! `Ext4Type` implements the `FsType` trait so the VFS layer can
//! discover and mount ext2 volumes by name (`"ext2"`).

use aster_systree::SysNode;
use device_id::DeviceId;

use super::{fs::Ext4, prelude::*};
use crate::fs::vfs::{
    file_system::FileSystem,
    registry::{FsCache, FsCreationCtx, FsProperties, FsType},
};

/// VFS-visible Ext4 filesystem type.
pub(in crate::fs) struct Ext4Type {
    cache: FsCache<DeviceId>,
}

/// The VFS filesystem type descriptor for Ext4.
pub(in crate::fs) static EXT2_TYPE: Ext4Type = Ext4Type {
    cache: FsCache::new(),
};

impl FsType for Ext4Type {
    type Key = DeviceId;

    fn name(&self) -> &'static str {
        "ext2"
    }

    fn properties(&self) -> FsProperties {
        FsProperties::NEED_DISK
    }

    fn create(&self, fs_creation_ctx: &mut FsCreationCtx) -> Result<Arc<dyn FileSystem>> {
        let disk = fs_creation_ctx.resolve_block_device()?.clone();
        let flags = fs_creation_ctx.flags();
        let args = fs_creation_ctx.args();
        Ext4::open(disk, flags, args).map(|fs| fs as Arc<dyn FileSystem>)
    }

    fn obtain_key_and_cache(
        &self,
        fs_creation_ctx: &mut FsCreationCtx,
    ) -> Option<(DeviceId, &FsCache<DeviceId>)> {
        let key = fs_creation_ctx
            .resolve_block_device()
            .ok()
            .map(|disk| disk.id())?;

        Some((key, &self.cache))
    }

    fn sysnode(&self) -> Option<Arc<dyn SysNode>> {
        None
    }
}
