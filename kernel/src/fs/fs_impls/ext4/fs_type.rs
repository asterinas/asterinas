// SPDX-License-Identifier: MPL-2.0

//! VFS filesystem-type registration for ext4.
//!
//! `Ext4Type` implements the `FsType` trait so the VFS layer can discover and
//! mount ext4 volumes by name (`"ext4"`).

use aster_systree::SysNode;

use super::{Ext4, fs::MountFlavor};
use crate::{
    fs::vfs::{
        file_system::FileSystem,
        registry::{FsCreationCtx, FsProperties, FsType},
    },
    prelude::*,
};

/// VFS-visible ext2 filesystem type, served by the same driver.
///
/// Mirrors Linux, where the ext4 driver can serve `ext2` mounts
/// (`CONFIG_EXT4_USE_FOR_EXT2`): the flavor restricts mounting to true
/// ext2-format volumes and is reported back as the filesystem name.
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
        Ext4::open(disk, MountFlavor::Ext2, args).map(|fs| fs as Arc<dyn FileSystem>)
    }

    fn sysnode(&self) -> Option<Arc<dyn SysNode>> {
        None
    }
}
