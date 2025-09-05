// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_systree::singleton as systree_singleton;
use spin::Once;

use crate::{
    fs::{
        registry::{FsProperties, FsType},
        sysfs::inode::SysFsInode,
        utils::{systree_inode::SysTreeInodeTy, FileSystem, FsFlags, Inode, SuperBlock},
        Result,
    },
    prelude::*,
};

/// A file system for exposing kernel information to the user space.
#[derive(Debug)]
pub(super) struct SysFs {
    sb: SuperBlock,
    root: Arc<dyn Inode>,
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
        let sb = SuperBlock::new(MAGIC_NUMBER, BLOCK_SIZE, NAME_MAX);
        let systree_ref = systree_singleton();
        let root_inode = SysFsInode::new_root(systree_ref.root().clone());

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

pub(super) struct SysFsType;

impl FsType for SysFsType {
    fn name(&self) -> &'static str {
        "sysfs"
    }

    fn properties(&self) -> FsProperties {
        FsProperties::empty()
    }

    fn create(
        &self,
        _args: Option<CString>,
        _disk: Option<Arc<dyn aster_block::BlockDevice>>,
    ) -> Result<Arc<dyn FileSystem>> {
        Ok(SysFs::singleton().clone() as _)
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        None
    }
}
