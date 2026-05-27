// SPDX-License-Identifier: MPL-2.0

//! VFS filesystem-type registration for ext2.
//!
//! `Ext2Type` implements the `FsType` trait so the VFS layer can
//! discover and mount ext2 volumes by name (`"ext2"`).

use aster_systree::SysNode;

use super::{fs::Ext2, prelude::*};
use crate::fs::vfs::{
    file_system::FileSystem,
    registry::{FsCreationCtx, FsProperties, FsType},
};

/// VFS-visible Ext2 filesystem type.
pub(super) struct Ext2Type;

impl FsType for Ext2Type {
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

    fn sysnode(&self) -> Option<Arc<dyn SysNode>> {
        None
    }
}
