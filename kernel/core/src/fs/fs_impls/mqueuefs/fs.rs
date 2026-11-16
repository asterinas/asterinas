// SPDX-License-Identifier: MPL-2.0

//! The `mqueuefs` file system instance and its type marker.

use core::sync::atomic::{AtomicU64, Ordering};

use spin::Once;

use super::{BLOCK_SIZE, MQUEUE_MAGIC, NAME_MAX, ROOT_INO, inode::MqueueRootInode};
use crate::{
    fs::{
        pseudofs::AnonDeviceId,
        vfs::{
            file_system::{FileSystem, FsEventSubscriberStats, SuperBlock},
            inode::Inode,
            registry::{FsCreationCtx, FsProperties, FsType},
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

impl MqueueFs {
    /// Returns the `mqueuefs` singleton.
    pub(crate) fn singleton() -> &'static Arc<Self> {
        static SINGLETON: Once<Arc<MqueueFs>> = Once::new();
        SINGLETON.call_once(Self::new)
    }

    fn new() -> Arc<Self> {
        let anon_device_id =
            AnonDeviceId::acquire().expect("no device ID is available for mqueuefs");
        let sb = SuperBlock::new(MQUEUE_MAGIC, BLOCK_SIZE, NAME_MAX, anon_device_id.id());
        Arc::new_cyclic(|weak_fs| Self {
            _anon_device_id: anon_device_id,
            sb: sb.clone(),
            root: MqueueRootInode::new_root(weak_fs.clone(), &sb),
            inode_allocator: AtomicU64::new(ROOT_INO + 1),
            fs_event_subscriber_stats: FsEventSubscriberStats::new(),
        })
    }

    fn alloc_id(&self) -> u64 {
        self.inode_allocator.fetch_add(1, Ordering::Relaxed)
    }
}

impl FileSystem for MqueueFs {
    fn name(&self) -> &'static str {
        "mqueue"
    }

    fn sync(&self) -> Result<()> {
        // `mqueuefs` is a pseudo file system; sync is a no-op.
        Ok(())
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }

    fn sb(&self) -> SuperBlock {
        self.sb.clone()
    }

    fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats {
        &self.fs_event_subscriber_stats
    }
}

/// The registered type of the `mqueuefs` file system.
pub(crate) struct MqueueFsType;

impl FsType for MqueueFsType {
    type Key = ();

    fn name(&self) -> &'static str {
        "mqueue"
    }

    fn properties(&self) -> FsProperties {
        FsProperties::empty()
    }

    fn create(&self, _fs_creation_ctx: &mut FsCreationCtx) -> Result<Arc<dyn FileSystem>> {
        Ok(MqueueFs::singleton().clone())
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        None
    }
}
