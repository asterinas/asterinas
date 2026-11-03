// SPDX-License-Identifier: MPL-2.0

//! securityfs filesystem type and singleton mount instance.
//!
//! [`SecurityFs`] owns the superblock and exposes the root node created by the
//! neighboring `node` module through the VFS.

use aster_systree::SysNode;
use spin::Once;

use super::{inode::SecurityFsInode, node::SecurityRootNode};
use crate::{
    fs::{
        pseudofs::AnonDeviceId,
        utils::systree_inode::SysTreeInodeTy,
        vfs::{
            file_system::{FileSystem, FsEventSubscriberStats, SuperBlock},
            inode::Inode,
            registry::{FsCreationCtx, FsProperties, FsType},
        },
    },
    prelude::*,
};

#[derive(Debug)]
pub(super) struct SecurityFs {
    _anon_device_id: AnonDeviceId,
    sb: SuperBlock,
    root: Arc<dyn Inode>,
    fs_event_subscriber_stats: FsEventSubscriberStats,
}

const MAGIC_NUMBER: u64 = 0x73636673;
const BLOCK_SIZE: usize = 4096;
const NAME_MAX: usize = 255;

impl SecurityFs {
    pub(super) fn singleton() -> &'static Arc<Self> {
        static SINGLETON: Once<Arc<SecurityFs>> = Once::new();

        SINGLETON.call_once(Self::new)
    }

    fn new() -> Arc<Self> {
        let anon_device_id =
            AnonDeviceId::acquire().expect("no device ID is available for securityfs");
        let sb = SuperBlock::new(MAGIC_NUMBER, BLOCK_SIZE, NAME_MAX, anon_device_id.id());
        let root = SecurityFsInode::new_root(SecurityRootNode::singleton().clone(), &sb);

        Arc::new(Self {
            _anon_device_id: anon_device_id,
            sb,
            root,
            fs_event_subscriber_stats: FsEventSubscriberStats::new(),
        })
    }
}

impl FileSystem for SecurityFs {
    fn name(&self) -> &'static str {
        "securityfs"
    }

    fn sync(&self) -> Result<()> {
        Ok(())
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }

    fn sb(&self) -> SuperBlock {
        self.sb.clone()
    }

    fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats {
        &self.fs_event_subscriber_stats
    }
}

pub(super) struct SecurityFsType;

impl FsType for SecurityFsType {
    type Key = ();

    fn name(&self) -> &'static str {
        "securityfs"
    }

    fn properties(&self) -> FsProperties {
        FsProperties::empty()
    }

    fn create(&self, _fs_creation_ctx: &mut FsCreationCtx) -> Result<Arc<dyn FileSystem>> {
        Ok(SecurityFs::singleton().clone())
    }

    fn sysnode(&self) -> Option<Arc<dyn SysNode>> {
        None
    }
}
