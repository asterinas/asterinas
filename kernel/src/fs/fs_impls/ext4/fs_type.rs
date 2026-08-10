// SPDX-License-Identifier: MPL-2.0

//! VFS filesystem-type registration for ext4.
//!
//! `Ext4Type` implements the `FsType` trait so the VFS layer can discover and
//! mount ext4 volumes by name (`"ext4"`).

use aster_systree::SysNode;

use super::Ext4;
use crate::{
    fs::vfs::{
        file_system::FileSystem,
        registry::{FsCreationCtx, FsProperties, FsType},
    },
    prelude::*,
};

/// VFS-visible ext4 filesystem type.
pub(super) struct Ext4Type;

impl FsType for Ext4Type {
    fn name(&self) -> &'static str {
        "ext4"
    }

    fn properties(&self) -> FsProperties {
        FsProperties::NEED_DISK
    }

    fn create(&self, fs_creation_ctx: &FsCreationCtx) -> Result<Arc<dyn FileSystem>> {
        let disk = fs_creation_ctx.resolve_block_device()?;
        Ext4::open(disk).map(|fs| fs as Arc<dyn FileSystem>)
    }

    fn sysnode(&self) -> Option<Arc<dyn SysNode>> {
        None
    }
}
