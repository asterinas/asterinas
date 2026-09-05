// SPDX-License-Identifier: MPL-2.0

//! The VFS registration type of overlayfs.
//!
//! [`OverlayFsType`] is the carrier the VFS registry sees: it publishes the
//! filesystem name `overlay`, and each mount request under that name is
//! answered by constructing one [`OverlayFs`] from the mount's creation
//! context. Registration happens once at filesystem initialization; every
//! later mount builds its state through that construction call.

use crate::{
    fs::{
        fs_impls::overlayfs::fs::OverlayFs,
        vfs::{
            file_system::FileSystem,
            registry::{FsCreationCtx, FsProperties, FsType},
        },
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
        let overlay_fs = OverlayFs::new(fs_creation_ctx)?;
        Ok(overlay_fs)
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        None
    }
}
