// SPDX-License-Identifier: MPL-2.0

//! Mount resource and policy: the VFS entry point, the filesystem object,
//! and the read-only snapshots published to the rest of the overlayfs
//! implementation.
//!
//! This module provides the VFS entry point ([`OverlayFsType`] implementing
//! `crate::fs::vfs::registry::FsType`), the top-level overlay filesystem object
//! ([`OverlayFs`]), and the read-only snapshots consumed by sibling modules
//! (`OverlayLayerStack`/`OverlayLayer`/`RealPath`, `MountPolicy`,
//! `CreatorCredentialPolicy`, `UpperFilesystemCapabilities`,
//! `WriteAccessAccounting`, `UpperWorkdirClaim`). All fallible mount work
//! happens inside `FsType::create` → `OverlayFs::new`; the only values that
//! cross this module boundary outward are an `Arc<dyn FileSystem>` and an
//! `Errno`-encoded error result.

mod build;
mod claims;
mod layers;
mod options;
mod policy;
mod superblock;

pub(in crate::fs::fs_impls::overlayfs) use layers::RealPath;
pub(in crate::fs::fs_impls::overlayfs) use options::XinoMode;
use ostd::task::Task;
pub(super) use superblock::OverlayFs;

use crate::{
    fs::vfs::{
        file_system::FileSystem,
        registry::{FsCreationCtx, FsProperties, FsType},
    },
    prelude::*,
    process::posix_thread::{AsPosixThread, PosixThread},
};

/// The external-facing filesystem name of overlayfs (mirrors Linux
/// `ovl_fs_type`).
///
/// Single representation of the `"overlay"` name used by the VFS entry point
/// ([`FsType::name`]), the reported mount-source default (`build.rs`), and
/// [`FileSystem::name`]
/// (`superblock.rs`).
pub(super) const OVERLAY_FS_NAME: &str = "overlay";

/// The VFS entry point of the overlay filesystem (mirrors Linux `ovl_fs_type`).
///
/// Registered by [`super::init`] as the active overlay filesystem entry point.
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

/// Runs `operation_fn` with the current task's POSIX thread.
///
/// Overlay mount construction (`OverlayFs::new`) executes synchronously
/// inside the mounting task's syscall (`mount(2)`/`fsconfig(2)`); both
/// `FsCreationCtx::new` callers are syscall handlers, so the current POSIX
/// thread is exactly the mounting thread whose context upstream previously
/// carried in `FsCreationCtx::task_ctx`. The rootfs direct-boot path
/// (`FsCreationCtx::from_block_device`) never constructs an overlay (overlay
/// is not a rootfs candidate, `rootfs.rs::SUPPORTED_ROOTFS_TYPES`), so the
/// `None` branches below are defensive and fail the mount closed.
pub(super) fn with_current_posix_thread<T>(
    operation_fn: impl FnOnce(&PosixThread) -> Result<T>,
) -> Result<T> {
    let current_task = Task::current().ok_or_else(|| {
        Error::with_message(Errno::EINVAL, "the overlay mount has no current task")
    })?;
    let posix_thread = current_task.as_posix_thread().ok_or_else(|| {
        Error::with_message(
            Errno::EINVAL,
            "the overlay mount task is not a POSIX thread",
        )
    })?;
    operation_fn(posix_thread)
}
