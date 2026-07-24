// SPDX-License-Identifier: MPL-2.0

//! VFS filesystem-type registration for ext2.
//!
//! `Ext2Type` implements the `FsType` trait so the VFS layer can
//! discover and mount ext2 volumes by name (`"ext2"`).

use aster_systree::SysNode;

use super::{fs::Ext2, prelude::*, super_block::MAGIC_NUM};
use crate::fs::{
    utils::NAME_MAX,
    vfs::{
        registry::{FsCreationCtx, FsProperties, FsType},
        super_block::SuperBlock,
    },
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

    fn create(&self, fs_creation_ctx: &FsCreationCtx) -> Result<Arc<SuperBlock>> {
        let disk = fs_creation_ctx.resolve_block_device()?;
        let args = fs_creation_ctx.args();
        let fs = Ext2::open(disk, args)?;
        let container_device_id = fs.container_device_id();
        Ok(SuperBlock::new(
            fs,
            MAGIC_NUM as u64,
            BLOCK_SIZE,
            NAME_MAX,
            container_device_id,
        ))
    }

    fn sysnode(&self) -> Option<Arc<dyn SysNode>> {
        None
    }
}
