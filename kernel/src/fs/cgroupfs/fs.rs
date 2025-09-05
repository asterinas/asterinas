// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_block::BlockDevice;
use spin::Once;

use super::inode::CgroupInode;
use crate::{
    fs::{
        cgroupfs::systree_node::CgroupSystem,
        registry::{FsProperties, FsType},
        utils::{systree_inode::SysTreeInodeTy, FileSystem, FsFlags, Inode, SuperBlock},
        Result,
    },
    prelude::*,
};

/// A file system for managing cgroups.
pub(super) struct CgroupFs {
    sb: SuperBlock,
    root: Arc<dyn Inode>,
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
        let sb = SuperBlock::new(MAGIC_NUMBER, BLOCK_SIZE, NAME_MAX);
        let root_inode = CgroupInode::new_root(root_node);

        Arc::new(Self {
            sb,
            root: root_inode,
        })
    }
}

impl FileSystem for CgroupFs {
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

    fn flags(&self) -> FsFlags {
        FsFlags::empty()
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
        _args: Option<CString>,
        _disk: Option<Arc<dyn BlockDevice>>,
    ) -> Result<Arc<dyn FileSystem>> {
        Ok(CgroupFs::singleton().clone() as _)
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        Some(CgroupSystem::singleton().clone() as _)
    }
}
