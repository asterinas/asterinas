// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_block::BlockDevice;
use aster_systree::SysNode;
use spin::Once;

use super::inode::ConfigInode;
use crate::{
    fs::{
        Result,
        configfs::systree_node::ConfigRootNode,
        registry::{FsProperties, FsType},
        utils::{
            FileSystem, FsEventSubscriberStats, FsFlags, Inode, SuperBlock,
            systree_inode::SysTreeInodeTy,
        },
    },
    prelude::*,
};

/// A file system that provides a user-space interface for configuring kernel objects.
///
/// `ConfigFs` is a RAM-based file system that allows user-space applications to create,
/// configure, and manage kernel objects through a virtual file system interface.
/// Unlike sysfs which is primarily read-only and represents existing kernel state,
/// `ConfigFs` is designed for dynamic creation and configuration of kernel objects.
pub struct ConfigFs {
    sb: SuperBlock,
    root: Arc<dyn Inode>,
    fs_event_subscriber_stats: FsEventSubscriberStats,
}

// Magic number for `ConfigFs` (taken from Linux).
const MAGIC_NUMBER: u64 = 0x62656570;
const BLOCK_SIZE: usize = 4096;
const NAME_MAX: usize = 255;

impl ConfigFs {
    /// Returns the `CgroupFs` singleton.
    pub(super) fn singleton() -> &'static Arc<ConfigFs> {
        static SINGLETON: Once<Arc<ConfigFs>> = Once::new();

        SINGLETON.call_once(|| Self::new(ConfigRootNode::singleton().clone()))
    }

    fn new(root_node: Arc<ConfigRootNode>) -> Arc<Self> {
        let sb = SuperBlock::new(MAGIC_NUMBER, BLOCK_SIZE, NAME_MAX);
        let root_inode = ConfigInode::new_root(root_node);

        Arc::new(Self {
            sb,
            root: root_inode,
            fs_event_subscriber_stats: FsEventSubscriberStats::new(),
        })
    }
}

impl FileSystem for ConfigFs {
    fn name(&self) -> &'static str {
        "configfs"
    }

    fn sync(&self) -> Result<()> {
        // `ConfigFs` is volatile, sync is a no-op
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

pub(super) struct ConfigFsType;

impl FsType for ConfigFsType {
    fn name(&self) -> &'static str {
        "configfs"
    }

    fn properties(&self) -> FsProperties {
        FsProperties::empty()
    }

    fn create(
        &self,
        _flags: FsFlags,
        _args: Option<CString>,
        _disk: Option<Arc<dyn BlockDevice>>,
    ) -> Result<Arc<dyn FileSystem>> {
        Ok(ConfigFs::singleton().clone() as _)
    }

    fn sysnode(&self) -> Option<Arc<dyn SysNode>> {
        None
    }
}
