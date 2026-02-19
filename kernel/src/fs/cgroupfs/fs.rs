// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_block::BlockDevice;
use aster_systree::EmptyNode;
use spin::Once;

use super::inode::CgroupInode;
use crate::{
    fs::{
        Result,
        cgroupfs::systree_node::CgroupSystem,
        pseudofs,
        registry::{FsProperties, FsType},
        utils::{
            FileSystem, FsEventSubscriberStats, FsFlags, Inode, SuperBlock,
            systree_inode::SysTreeInodeTy,
        },
    },
    prelude::*,
};

/// A file system for managing cgroups.
pub(super) struct CgroupFs {
    sb: SuperBlock,
    root: Arc<dyn Inode>,
    fs_event_subscriber_stats: FsEventSubscriberStats,
}

// Magic number for cgroupfs v2 (taken from Linux)
const MAGIC_NUMBER: u64 = 0x63677270;
const BLOCK_SIZE: usize = 4096;
const NAME_MAX: usize = 255;

impl CgroupFs {
    /// Returns the `CgroupFs` singleton.
    pub(super) fn singleton() -> &'static Arc<CgroupFs> {
        static SINGLETON: Once<Arc<CgroupFs>> = Once::new();

        SINGLETON.call_once(|| Self::new(CgroupSystem::singleton().clone()))
    }

    fn new(root_node: Arc<CgroupSystem>) -> Arc<Self> {
        let dev_id = pseudofs::DEVICE_ID_ALLOCATOR
            .get()
            .unwrap()
            .allocate()
            .expect("no device ID is available for cgroupfs");
        let sb = SuperBlock::new(MAGIC_NUMBER, BLOCK_SIZE, NAME_MAX, dev_id);
        let root_inode = CgroupInode::new_root(root_node, dev_id);

        Arc::new(Self {
            sb,
            root: root_inode,
            fs_event_subscriber_stats: FsEventSubscriberStats::new(),
        })
    }
}

impl FileSystem for CgroupFs {
    fn name(&self) -> &'static str {
        "cgroup2"
    }

    fn sync(&self) -> Result<()> {
        // CgroupFs is volatile, sync is a no-op
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

pub(super) struct CgroupFsType;

impl FsType for CgroupFsType {
    fn name(&self) -> &'static str {
        "cgroup2"
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
        Ok(CgroupFs::singleton().clone() as _)
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        Some(EmptyNode::new("cgroup".into()))
    }
}
