// SPDX-License-Identifier: MPL-2.0

use spin::Once;

use crate::{
    fs::{
        pseudofs::AnonDeviceId,
        sysfs::{self, inode::SysFsInode},
        utils::systree_inode::SysTreeInodeTy,
        vfs::{
            file_system::{FileSystem, FsEventSubscriberStats, FsStats},
            inode::Inode,
            registry::{FsCreationCtx, FsProperties, FsType},
            super_block::SuperBlock,
        },
    },
    prelude::*,
};

/// A file system for exposing kernel information to the user space.
#[derive(Debug)]
pub(super) struct SysFs {
    anon_device_id: AnonDeviceId,
    root: Arc<dyn Inode>,
    fs_event_subscriber_stats: FsEventSubscriberStats,
}

const MAGIC_NUMBER: u64 = 0x62656572; // SYSFS_MAGIC
const BLOCK_SIZE: usize = 4096;
const NAME_MAX: usize = 255;

impl SysFs {
    /// Returns the `SysFs` singleton.
    pub(super) fn singleton() -> &'static Arc<SysFs> {
        static SINGLETON: Once<Arc<SysFs>> = Once::new();

        SINGLETON.call_once(Self::new)
    }

    #[cfg(ktest)]
    pub(super) fn new_for_ktest() -> Arc<Self> {
        Self::new()
    }

    fn new() -> Arc<Self> {
        let anon_device_id = AnonDeviceId::acquire().expect("no device ID is available for sysfs");
        let systree_ref = sysfs::systree_singleton();
        let root_inode = SysFsInode::new_root(systree_ref.root().clone(), anon_device_id.id());

        Arc::new(Self {
            anon_device_id,
            root: root_inode,
            fs_event_subscriber_stats: FsEventSubscriberStats::new(),
        })
    }

    fn into_super_block(self: Arc<Self>) -> Arc<SuperBlock> {
        let container_device_id = self.anon_device_id.id();
        SuperBlock::new(
            self,
            MAGIC_NUMBER,
            BLOCK_SIZE,
            NAME_MAX,
            container_device_id,
        )
    }
}

impl FileSystem for SysFs {
    fn name(&self) -> &'static str {
        "sysfs"
    }

    fn sync(&self) -> Result<()> {
        // `SysFs` is volatile, sync is a no-op
        Ok(())
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }

    fn stats(&self) -> FsStats {
        FsStats::default()
    }

    fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats {
        &self.fs_event_subscriber_stats
    }
}

pub(super) struct SysFsType;

impl FsType for SysFsType {
    fn name(&self) -> &'static str {
        "sysfs"
    }

    fn properties(&self) -> FsProperties {
        FsProperties::empty()
    }

    fn create(&self, _fs_creation_ctx: &FsCreationCtx) -> Result<Arc<SuperBlock>> {
        static SUPER_BLOCK: Once<Arc<SuperBlock>> = Once::new();

        Ok(SUPER_BLOCK
            .call_once(|| SysFs::singleton().clone().into_super_block())
            .clone())
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        None
    }
}
