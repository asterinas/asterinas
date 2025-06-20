// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_systree::SysBranchNode;

use super::inode::ConfigInode;
use crate::fs::{
    utils::{FileSystem, FsFlags, Inode, KernelFsInode, SuperBlock},
    Result,
};

/// A file system that can act as a manager of kernel objects
pub struct ConfigFs {
    sb: SuperBlock,
    root: Arc<dyn Inode>,
}

// Magic number for configfs (taken from Linux)
const MAGIC_NUMBER: u64 = 0x62656570;
const BLOCK_SIZE: usize = 4096;
const NAME_MAX: usize = 255;

impl ConfigFs {
    pub(super) fn new(root_node: Arc<dyn SysBranchNode>) -> Arc<Self> {
        let sb = SuperBlock::new(MAGIC_NUMBER, BLOCK_SIZE, NAME_MAX);
        let root_inode = ConfigInode::new_root(root_node);

        Arc::new(Self {
            sb,
            root: root_inode,
        })
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
