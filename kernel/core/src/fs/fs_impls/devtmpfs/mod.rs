// SPDX-License-Identifier: MPL-2.0

//! The device temporary filesystem.
//!
//! This module implements `devtmpfs` as a singleton filesystem backed by
//! `tmpfs`. Device subsystems submit node-management requests through
//! [`create_node`] and [`delete_node`]; the requests are serialized and handled
//! by the dedicated kernel thread `devtmpfsd`. This gives node operations a
//! single VFS execution context instead of mutating the filesystem tree directly
//! from arbitrary device-registration contexts, and keeps these operations
//! independent of the credentials of the device-registration caller, following
//! Linux's devtmpfs design.
//!
//! Mounting policy follows the selected init path. If an initramfs init is
//! selected, either by `rdinit=` or by the default `/init` lookup, the kernel
//! does not mount devtmpfs automatically; the selected initramfs init program is
//! responsible for mounting it if needed. Otherwise, Asterinas boots from the
//! configured root filesystem and the kernel mounts this singleton on `/dev`
//! during first-process initialization.

mod fs;
mod tree;
mod worker;

use alloc::borrow::Cow;

use device_id::DeviceId;
pub(in crate::fs) use fs::singleton;
pub(crate) use worker::{create_node, delete_node};

use crate::{
    device::DeviceType,
    fs::{
        file::{InodeMode, mkmod},
        vfs::path,
    },
    prelude::*,
};

/// The metadata that describes a device inode in devtmpfs.
///
/// The metadata contains the inode path relative to `/dev` and the permission
/// bits used when creating the inode. Device subsystems can use this type to
/// override the default mode.
///
/// If a device does not specify a mode explicitly, we use `mkmod!(u+rw)`,
/// matching Linux devtmpfs's default device inode permissions.
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/drivers/base/devtmpfs.c#L11>.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DevtmpfsInodeMeta {
    path: Cow<'static, str>,
    mode: InodeMode,
}

impl DevtmpfsInodeMeta {
    /// Creates the metadata for a devtmpfs inode with the default mode (`u+rw`).
    pub(crate) fn new(path: impl Into<Cow<'static, str>>) -> Result<Self> {
        Self::with_mode(path, mkmod!(u+rw))
    }

    /// Creates the metadata for a devtmpfs inode with the specified path and mode.
    pub(crate) fn with_mode(path: impl Into<Cow<'static, str>>, mode: InodeMode) -> Result<Self> {
        let path = path.into();
        if path.is_empty()
            || path.starts_with('/')
            || path
                .split('/')
                .any(|component| component.is_empty() || path::is_dot_or_dotdot(component))
        {
            return_errno_with_message!(Errno::EINVAL, "the device path is invalid");
        }
        Ok(Self { path, mode })
    }

    /// Returns the device inode path relative to `/dev`.
    pub(crate) fn path(&self) -> &str {
        &self.path
    }

    /// Returns the permission bits of the device inode.
    pub(crate) fn mode(&self) -> InodeMode {
        self.mode
    }
}

/// The complete description of a device node managed by devtmpfs.
pub(crate) struct DevtmpfsNode {
    device_type: DeviceType,
    device_id: DeviceId,
    meta: DevtmpfsInodeMeta,
}

impl DevtmpfsNode {
    pub(crate) fn new(
        device_type: DeviceType,
        device_id: DeviceId,
        meta: DevtmpfsInodeMeta,
    ) -> Self {
        Self {
            device_type,
            device_id,
            meta,
        }
    }
}

pub(super) fn init() {
    fs::init();
}

pub(super) fn init_in_first_kthread() {
    worker::init();
}
