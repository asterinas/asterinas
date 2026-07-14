// SPDX-License-Identifier: MPL-2.0

//! Filesystem registration and the singleton tmpfs backing for devtmpfs.

use spin::Once;

use crate::{
    fs::{
        tmpfs::TmpFs,
        vfs::{
            file_system::FileSystem,
            inode::RevalidationPolicy,
            registry::{FsCreationCtx, FsProperties, FsType},
        },
    },
    prelude::*,
};

pub(in crate::fs) fn singleton() -> &'static Arc<TmpFs> {
    static SINGLETON: Once<Arc<TmpFs>> = Once::new();

    SINGLETON.call_once(|| {
        // devtmpfsd creates and deletes device nodes from a kernel thread,
        // outside the VFS path operation that may have cached the dentry.
        // Revalidate directory entries so cached positive/negative dentries
        // reflect the latest devtmpfs tree.
        TmpFs::new_tmpfs_backing(
            "devtmpfs",
            RevalidationPolicy::REVALIDATE_EXISTS | RevalidationPolicy::REVALIDATE_ABSENT,
        )
    })
}

pub(super) fn init() {
    crate::fs::vfs::registry::register(&DevTmpFsType).unwrap();
}

struct DevTmpFsType;

impl FsType for DevTmpFsType {
    type Key = ();

    fn name(&self) -> &'static str {
        "devtmpfs"
    }

    fn properties(&self) -> FsProperties {
        FsProperties::empty()
    }

    fn create(&self, _fs_creation_ctx: &mut FsCreationCtx) -> Result<Arc<dyn FileSystem>> {
        Ok(singleton().clone())
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        None
    }
}
