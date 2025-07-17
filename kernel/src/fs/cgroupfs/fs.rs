// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_block::BlockDevice;
use aster_systree::SysBranchNode;

use super::inode::CgroupInode;
use crate::{
    context::Context,
    fs::{
        cgroupfs::systree_node::CgroupSystem,
        registry::{FsProperties, FsType},
        utils::{systree_inode::SysTreeInodeTy, FileSystem, FsFlags, Inode, SuperBlock},
        Result,
    },
    prelude::*,
};

/// A file system for managing cgroups.
pub struct CgroupFs {
    sb: SuperBlock,
    root: Arc<dyn Inode>,
}

// Magic number for cgroupfs v2 (taken from Linux)
const MAGIC_NUMBER: u64 = 0x63677270;
const BLOCK_SIZE: usize = 4096;
const NAME_MAX: usize = 255;

impl CgroupFs {
    pub(super) fn new(root_node: Arc<CgroupSystem>) -> Arc<Self> {
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

pub(super) struct CgroupFsType {
    systree_root: Arc<CgroupSystem>,
}

impl CgroupFsType {
    pub(super) fn new(systree_root: Arc<CgroupSystem>) -> Arc<Self> {
        Arc::new(Self { systree_root })
    }
}

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
        _ctx: &Context,
    ) -> Result<Arc<dyn FileSystem>> {
        if super::CGROUP_SINGLETON.is_completed() {
            return_errno_with_message!(Errno::EBUSY, "the cgroupfs has been created");
        }

        let cgroupfs = CgroupFs::new(self.systree_root.clone());
        super::CGROUP_SINGLETON.call_once(|| cgroupfs.clone());

        Ok(cgroupfs)
    }

    fn sysnode(&self) -> Option<Arc<dyn SysBranchNode>> {
        Some(self.systree_root.clone())
    }
}
