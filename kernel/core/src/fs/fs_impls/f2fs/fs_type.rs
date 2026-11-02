// SPDX-License-Identifier: MPL-2.0

//! VFS filesystem-type registration for F2FS.
//!
//! `F2fsType` implements the `FsType` trait so that the VFS layer can
//! discover and mount F2FS volumes by name (`"f2fs"`).

use aster_systree::SysNode;
use device_id::DeviceId;

use super::{fs::F2fs, prelude::*};
use crate::fs::vfs::{
    file_system::FileSystem,
    registry::{FsCache, FsCreationCtx, FsProperties, FsType},
};

/// VFS-visible F2FS filesystem type.
pub(in crate::fs) struct F2fsType {
    cache: FsCache<DeviceId>,
}

/// The VFS filesystem type descriptor for F2FS.
pub(in crate::fs) static F2FS_TYPE: F2fsType = F2fsType {
    cache: FsCache::new(),
};

impl FsType for F2fsType {
    type Key = DeviceId;

    fn name(&self) -> &'static str {
        "f2fs"
    }

    fn properties(&self) -> FsProperties {
        FsProperties::NEED_DISK
    }

    fn create(&self, fs_creation_ctx: &mut FsCreationCtx) -> Result<Arc<dyn FileSystem>> {
        let disk = fs_creation_ctx.resolve_block_device()?.clone();
        let flags = fs_creation_ctx.flags();
        let args = fs_creation_ctx.args();
        F2fs::open(disk, flags, args).map(|fs| fs as Arc<dyn FileSystem>)
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
