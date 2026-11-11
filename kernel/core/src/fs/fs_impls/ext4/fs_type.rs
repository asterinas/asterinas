// SPDX-License-Identifier: MPL-2.0

//! VFS filesystem-type registration for ext2 and ext4.

use aster_systree::SysNode;
use device_id::DeviceId;

use super::{
    fs::{Ext4, MountFlavor},
    prelude::*,
};
use crate::fs::vfs::{
    file_system::FileSystem,
    registry::{FsCache, FsCreationCtx, FsProperties, FsType},
};

pub(in crate::fs) struct ExtFsType {
    cache: FsCache<DeviceId>,
    name: &'static str,
    flavor: MountFlavor,
}

pub(in crate::fs) static EXT2_TYPE: ExtFsType = ExtFsType {
    cache: FsCache::new(),
    name: "ext2",
    flavor: MountFlavor::Ext2,
};

pub(in crate::fs) static EXT4_TYPE: ExtFsType = ExtFsType {
    cache: FsCache::new(),
    name: "ext4",
    flavor: MountFlavor::Ext4,
};

impl FsType for ExtFsType {
    type Key = DeviceId;

    fn name(&self) -> &'static str {
        self.name
    }

    fn properties(&self) -> FsProperties {
        FsProperties::NEED_DISK
    }

    fn create(&self, fs_creation_ctx: &mut FsCreationCtx) -> Result<Arc<dyn FileSystem>> {
        let disk = fs_creation_ctx.resolve_block_device()?.clone();
        let flags = fs_creation_ctx.flags();
        let args = fs_creation_ctx.args();
        Ext4::open(disk, flags, self.flavor, args).map(|fs| fs as Arc<dyn FileSystem>)
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
