// SPDX-License-Identifier: MPL-2.0

use super::options::{DEFAULT_ROOT_MODE, MountOptions};
use crate::{
    fs::{
        file::InodeMode,
        fs_impls::ramfs::{self, RamFs},
        pseudofs::AnonDeviceId,
        vfs::{
            file_system::{FileSystem, SuperBlock},
            inode::RevalidationPolicy,
            registry::{FsCreationCtx, FsProperties, FsType},
        },
    },
    prelude::*,
    process::{Gid, Uid},
};

const TMPFS_MAGIC: u64 = 0x0102_1994;

/// The temporary file system (tmpfs) structure.
//
// TODO: `TmpFs` currently aliases `RamFs` and relies on `RamFs::new_tmpfs_backing()`
// to create tmpfs-flavored ramfs instances. In the future we need to
// implement a dedicated tmpfs with tmpfs-specific features such as memory
// limits and swap support.
pub(crate) type TmpFs = RamFs;

impl TmpFs {
    /// Creates a tmpfs filesystem instance whose root directory is owned by root.
    pub(in crate::fs) fn new_tmpfs() -> Arc<Self> {
        Self::new_tmpfs_with_root(DEFAULT_ROOT_MODE, Uid::new_root(), Gid::new_root())
    }

    /// Creates a tmpfs filesystem instance whose root directory has the given mode and owner.
    fn new_tmpfs_with_root(root_mode: InodeMode, root_uid: Uid, root_gid: Gid) -> Arc<Self> {
        Self::new_tmpfs_backing(
            "tmpfs",
            RevalidationPolicy::empty(),
            root_mode,
            root_uid,
            root_gid,
        )
    }

    /// Creates a tmpfs-backed filesystem instance with a custom filesystem name,
    /// directory-entry revalidation policy, and root directory mode and owner.
    pub(in crate::fs) fn new_tmpfs_backing(
        name: &'static str,
        revalidation_policy: RevalidationPolicy,
        root_mode: InodeMode,
        root_uid: Uid,
        root_gid: Gid,
    ) -> Arc<Self> {
        let anon_device_id = AnonDeviceId::acquire().expect("no device ID is available for tmpfs");
        let sb = {
            let mut super_block = SuperBlock::new(
                TMPFS_MAGIC,
                ramfs::BLOCK_SIZE,
                ramfs::NAME_MAX,
                anon_device_id.id(),
            );
            let max_blocks = default_max_blocks();
            let max_inodes = default_max_inodes();
            super_block.blocks = max_blocks;
            super_block.bfree = max_blocks;
            super_block.bavail = max_blocks;
            super_block.files = max_inodes;
            super_block.ffree = max_inodes;
            super_block
        };
        Self::new_with_sb(
            name,
            anon_device_id,
            sb,
            revalidation_policy,
            root_mode,
            root_uid,
            root_gid,
        )
    }
}

// FIXME: These defaults are only a rough approximation for tmpfs-over-ramfs.
// A dedicated tmpfs implementation should replace them with real tmpfs limit
// and accounting semantics.
fn default_max_blocks() -> usize {
    crate::vm::mem_total() / PAGE_SIZE / 2
}
fn default_max_inodes() -> usize {
    crate::vm::mem_total() / PAGE_SIZE / 2
}

pub(super) struct TmpFsType;

impl FsType for TmpFsType {
    type Key = ();

    fn name(&self) -> &'static str {
        "tmpfs"
    }

    fn properties(&self) -> FsProperties {
        FsProperties::empty()
    }

    fn create(&self, fs_creation_ctx: &mut FsCreationCtx) -> Result<Arc<dyn FileSystem>> {
        let options = MountOptions::parse(fs_creation_ctx.args())?;
        Ok(TmpFs::new_tmpfs_with_root(
            options.root_mode,
            options.root_uid,
            options.root_gid,
        ))
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        None
    }
}
