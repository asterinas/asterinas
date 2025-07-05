// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_systree::SysBranchNode;

use super::inode::CgroupInode;
use crate::fs::{
    utils::{systree_inode::SysTreeInodeTy, FileSystem, FsFlags, Inode, SuperBlock},
    Result,
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
    pub(super) fn new(root_node: Arc<dyn SysBranchNode>) -> Arc<Self> {
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
