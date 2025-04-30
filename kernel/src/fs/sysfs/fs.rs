// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_systree::singleton as systree_singleton;

use crate::fs::{
    sysfs::inode::SysFsInode,
    utils::{FileSystem, FsFlags, Inode, SuperBlock},
    Result,
};

/// A file system for exposing kernel information to the user space.
#[derive(Debug)]
pub struct SysFs {
    sb: SuperBlock,
    root: Arc<dyn Inode>,
}

const MAGIC_NUMBER: u64 = 0x62656572; // SYSFS_MAGIC
const BLOCK_SIZE: usize = 4096;
const NAME_MAX: usize = 255;

impl SysFs {
    pub(crate) fn new() -> Arc<Self> {
        let sb = SuperBlock::new(MAGIC_NUMBER, BLOCK_SIZE, NAME_MAX);
        let systree_ref = systree_singleton();
        let root_inode = SysFsInode::new_root(systree_ref);

        Arc::new(Self {
            sb,
            root: root_inode,
        })
    }
}

impl FileSystem for SysFs {
    fn sync(&self) -> Result<()> {
        // Sysfs is volatile, sync is a no-op
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
