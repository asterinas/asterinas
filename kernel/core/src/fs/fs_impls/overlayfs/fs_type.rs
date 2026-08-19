// SPDX-License-Identifier: MPL-2.0

//! The overlayfs VFS registration carrier.

use crate::{
    fs::vfs::{
        file_system::FileSystem,
        registry::{FsCreationCtx, FsProperties, FsType},
    },
    prelude::*,
};

pub(super) const OVERLAY_FS_NAME: &str = "overlay";

pub(super) struct OverlayFsType;

impl FsType for OverlayFsType {
    type Key = ();

    fn name(&self) -> &'static str {
        OVERLAY_FS_NAME
    }

    fn properties(&self) -> FsProperties {
        FsProperties::empty()
    }

    fn create(&self, fs_creation_ctx: &mut FsCreationCtx) -> Result<Arc<dyn FileSystem>> {
        let overlay_fs =
            crate::fs::fs_impls::overlayfs::superblock::OverlayFs::new(fs_creation_ctx)?;
        Ok(overlay_fs)
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        None
    }
}
