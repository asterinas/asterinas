// SPDX-License-Identifier: MPL-2.0

//! EROFS creation through the existing VFS instance cache.
//!
//! `ErofsType` validates and normalizes requests before the private factory's
//! blanket implementation checks the cache. The cache owns only weak references
//! and retains the VFS root identity without a second EROFS instance registry.

use aster_systree::SysNode;
use device_id::DeviceId;

use super::{
    fs::{Erofs, ErofsOpenError},
    inode::desc::InodeDescError,
    ondisk::super_block::SuperBlockError,
};
use crate::{
    fs::vfs::{
        file_system::{FileSystem, FsFlags},
        registry::{DynFsType, FsAndRoot, FsCache, FsCreationCtx, FsProperties, FsType},
    },
    prelude::*,
};

pub(super) static EROFS_TYPE: ErofsType = ErofsType::new();

/// An EROFS creation entry that normalizes inherent read-only requests.
pub(super) struct ErofsType {
    factory: ErofsFactory,
}

impl ErofsType {
    /// Creates an entry with its own weak instance cache.
    pub(super) const fn new() -> Self {
        Self {
            factory: ErofsFactory {
                cache: FsCache::new(),
            },
        }
    }
}

impl DynFsType for ErofsType {
    fn name(&self) -> &'static str {
        <ErofsFactory as FsType>::name(&self.factory)
    }

    fn properties(&self) -> FsProperties {
        <ErofsFactory as FsType>::properties(&self.factory)
    }

    fn get_or_create(&self, ctx: &mut FsCreationCtx) -> Result<FsAndRoot> {
        // A live cache hit skips create, so every request is checked here first.
        let allowed_flags = FsFlags::RDONLY | FsFlags::SILENT;
        if !(ctx.flags() - allowed_flags).is_empty()
            || ctx.args().is_some_and(|args| !args.is_empty())
        {
            return Err(Error::with_message(
                Errno::EOPNOTSUPP,
                "unsupported erofs mount options",
            ));
        }

        let disk = ctx.resolve_block_device()?.clone();
        let flags = ctx.flags() | FsFlags::RDONLY;
        // SILENT remains a request hint; the instance reports only RDONLY.
        // The resolved device is independent of the borrowed, optional args.
        let mut normalized = FsCreationCtx::from_block_device(disk, flags, ctx.args());
        <ErofsFactory as DynFsType>::get_or_create(&self.factory, &mut normalized)
    }

    fn sysnode(&self) -> Option<Arc<dyn SysNode>> {
        <ErofsFactory as FsType>::sysnode(&self.factory)
    }
}

struct ErofsFactory {
    cache: FsCache<DeviceId>,
}

impl FsType for ErofsFactory {
    type Key = DeviceId;

    fn name(&self) -> &'static str {
        "erofs"
    }

    fn properties(&self) -> FsProperties {
        FsProperties::NEED_DISK
    }

    fn create(&self, ctx: &mut FsCreationCtx) -> Result<Arc<dyn FileSystem>> {
        let disk = ctx.resolve_block_device()?.clone();
        Ok(Erofs::open(disk).map_err(open_error)?)
    }

    fn obtain_key_and_cache(
        &self,
        ctx: &mut FsCreationCtx,
    ) -> Option<(DeviceId, &FsCache<DeviceId>)> {
        // The outer entry supplies a resolved context. If resolution ever fails,
        // None takes the blanket create path, whose Result propagates the error.
        let Ok(disk) = ctx.resolve_block_device() else {
            return None;
        };
        Some((disk.id(), &self.cache))
    }

    fn sysnode(&self) -> Option<Arc<dyn SysNode>> {
        None
    }
}

/// Projects internal opening failures at the production VFS boundary.
fn open_error(error: ErofsOpenError) -> Error {
    match error {
        ErofsOpenError::Projection(error)
        | ErofsOpenError::Cache(error)
        | ErofsOpenError::SuperBlock(SuperBlockError::Device(error))
        | ErofsOpenError::Root(InodeDescError::Device(error))
        | ErofsOpenError::Root(InodeDescError::Projection(error)) => Error::from(error),
        ErofsOpenError::SuperBlock(SuperBlockError::Corrupted(_))
        | ErofsOpenError::Root(InodeDescError::Corrupted(_))
        | ErofsOpenError::RootNotDirectory => {
            Error::with_message(Errno::EIO, "invalid erofs filesystem metadata")
        }
        ErofsOpenError::SuperBlock(SuperBlockError::Unsupported(_))
        | ErofsOpenError::Root(InodeDescError::Unsupported(_)) => {
            Error::with_message(Errno::EOPNOTSUPP, "unsupported erofs filesystem metadata")
        }
    }
}
