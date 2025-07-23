// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_block::BlockDevice;
use aster_systree::SysBranchNode;

use super::inode::ConfigInode;
use crate::{
    fs::{
        configfs::systree_node::ConfigRootNode,
        registry::{FsProperties, FsType},
        utils::{systree_inode::SysTreeInodeTy, FileSystem, FsFlags, Inode, SuperBlock},
        Result,
    },
    prelude::*,
};

/// A file system that can act as a manager of kernel objects
pub struct ConfigFs {
    sb: SuperBlock,
    root: Arc<dyn Inode>,
    systree_root: Arc<ConfigRootNode>,
}

// Magic number for configfs (taken from Linux)
const MAGIC_NUMBER: u64 = 0x62656570;
const BLOCK_SIZE: usize = 4096;
const NAME_MAX: usize = 255;

impl ConfigFs {
    pub(super) fn new(root_node: Arc<ConfigRootNode>) -> Arc<Self> {
        let sb = SuperBlock::new(MAGIC_NUMBER, BLOCK_SIZE, NAME_MAX);
        let root_inode = ConfigInode::new_root(root_node.clone());

        Arc::new(Self {
            sb,
            root: root_inode,
            systree_root: root_node,
        })
    }

    pub(super) fn systree_root(&self) -> &Arc<ConfigRootNode> {
        &self.systree_root
    }
}

impl FileSystem for ConfigFs {
    fn sync(&self) -> Result<()> {
        // ConfigFs is volatile, sync is a no-op
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

pub(super) struct ConfigFsType {
    systree_root: Arc<ConfigRootNode>,
}

impl ConfigFsType {
    pub(super) fn new(systree_root: Arc<ConfigRootNode>) -> Arc<Self> {
        Arc::new(Self { systree_root })
    }
}

impl FsType for ConfigFsType {
    fn name(&self) -> &'static str {
        "configfs"
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
        if super::CONFIG_SINGLETON.is_completed() {
            return_errno_with_message!(Errno::EBUSY, "the configfs has been created");
        }

        let configfs = ConfigFs::new(self.systree_root.clone());
        super::CONFIG_SINGLETON.call_once(|| configfs.clone());

        Ok(configfs)
    }

    fn sysnode(&self) -> Option<Arc<dyn SysBranchNode>> {
        None
    }
}
