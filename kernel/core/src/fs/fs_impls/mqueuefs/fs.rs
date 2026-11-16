// SPDX-License-Identifier: MPL-2.0

//! The `mqueuefs` file system instance and its type marker.

use core::sync::atomic::AtomicU64;

use crate::{
    fs::{
        pseudofs::AnonDeviceId,
        vfs::{
            file_system::{FsEventSubscriberStats, SuperBlock},
            inode::Inode,
        },
    },
    prelude::*,
};

/// A `mqueuefs` file system instance.
///
/// A single instance is shared by all mounts of `mqueuefs` (the singleton
/// pattern), mirroring how Linux exposes the global `/dev/mqueue`.
pub(crate) struct MqueueFs {
    /// Anonymous device ID for this pseudo file system.
    _anon_device_id: AnonDeviceId,
    /// The super block.
    sb: SuperBlock,
    /// Root inode of the file system.
    root: Arc<dyn Inode>,
    /// Inode number allocator.
    inode_allocator: AtomicU64,
    /// FS event subscriber stats for this file system.
    fs_event_subscriber_stats: FsEventSubscriberStats,
}

/// The registered type of the `mqueuefs` file system.
pub(crate) struct MqueueFsType;
