// SPDX-License-Identifier: MPL-2.0

use spin::Once;

use super::{inode::SecurityFsInode, systree_node::SecurityRootNode};
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

/// A pseudo filesystem that exposes kernel security interfaces.
pub struct SecurityFs {
    _anon_device_id: AnonDeviceId,
    sb: SuperBlock,
    root: Arc<dyn Inode>,
    fs_event_subscriber_stats: FsEventSubscriberStats,
}

// Reference: <https://elixir.bootlin.com/linux/v6.18.6/source/include/uapi/linux/magic.h>.
const MAGIC_NUMBER: u64 = 0x7363_6673;
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
        let root_inode = SecurityFsInode::new_root(SecurityRootNode::new(), &sb);

        Arc::new(Self {
            _anon_device_id: anon_device_id,
            sb,
            root: root_inode,
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

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        None
    }
}
