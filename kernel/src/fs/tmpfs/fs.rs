// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::{
        ramfs::RamFs,
        registry::{FsProperties, FsType},
        utils::{FileSystem, FsFlags, Inode, SuperBlock},
    },
    prelude::*,
};

/// The temporary file system (tmpfs) structure.
//
// TODO: Currently, tmpfs is implemented as a thin wrapper around RamFs.
// In the future we need to implement tmpfs-specific features such as
// memory limits and swap support.
pub struct TmpFs {
    inner: Arc<RamFs>,
}

impl FileSystem for TmpFs {
    fn sync(&self) -> Result<()> {
        // do nothing
        Ok(())
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.inner.root_inode()
    }

    fn sb(&self) -> SuperBlock {
        self.inner.sb()
    }

    fn flags(&self) -> FsFlags {
        FsFlags::DENTRY_UNEVICTABLE
    }
}

pub(super) struct TmpFsType;

impl FsType for TmpFsType {
    fn name(&self) -> &'static str {
        "tmpfs"
    }

    fn properties(&self) -> FsProperties {
        FsProperties::empty()
    }

    fn create(
        &self,
        _args: Option<CString>,
        _disk: Option<Arc<dyn aster_block::BlockDevice>>,
    ) -> Result<Arc<dyn FileSystem>> {
        Ok(Arc::new(TmpFs {
            inner: RamFs::new(),
        }))
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        None
    }
}
