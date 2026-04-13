// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use aster_block::BlockDevice;
use aster_systree::EmptyNode;
use ostd::task::Task;
use spin::Once;

use super::inode::CgroupInode;
use crate::{
    fs::{
        Result,
        pseudofs::AnonDeviceId,
        utils::systree_inode::SysTreeInodeTy,
        vfs::{
            file_system::{FileSystem, FsEventSubscriberStats, FsFlags, SuperBlock},
            inode::Inode,
            registry::{FsProperties, FsType},
        },
    },
    prelude::*,
    process::posix_thread::AsThreadLocal,
};

/// A file system for managing cgroups.
pub(super) struct CgroupFs {
    _anon_device_id: AnonDeviceId,
    sb: SuperBlock,
    fs_event_subscriber_stats: FsEventSubscriberStats,
}

// Magic number for cgroupfs v2 (taken from Linux)
const MAGIC_NUMBER: u64 = 0x63677270;
const BLOCK_SIZE: usize = 4096;
const NAME_MAX: usize = 255;

impl CgroupFs {
    /// Returns the `CgroupFs` singleton.
    pub(super) fn singleton() -> &'static Arc<CgroupFs> {
        static SINGLETON: Once<Arc<CgroupFs>> = Once::new();

        SINGLETON.call_once(Self::new)
    }

    fn new() -> Arc<Self> {
        let anon_device_id =
            AnonDeviceId::acquire().expect("no device ID is available for cgroupfs");
        let sb = SuperBlock::new(MAGIC_NUMBER, BLOCK_SIZE, NAME_MAX, anon_device_id.id());

        Arc::new(Self {
            _anon_device_id: anon_device_id,
            sb,
            fs_event_subscriber_stats: FsEventSubscriberStats::new(),
        })
    }
}

impl FileSystem for CgroupFs {
    fn name(&self) -> &'static str {
        "cgroup2"
    }

    fn sync(&self) -> Result<()> {
        // `CgroupFs` is volatile, sync is a no-op
        Ok(())
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        let current_task = Task::current().unwrap();
        let thread_local = current_task.as_thread_local().unwrap();
        let ns_proxy = thread_local.borrow_ns_proxy();
        let cgroup_namespace = ns_proxy.unwrap().cgroup_ns();

        CgroupInode::new_root(cgroup_namespace.root_node(), &self.sb)
    }

    fn sb(&self) -> SuperBlock {
        self.sb.clone()
    }

    fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats {
        &self.fs_event_subscriber_stats
    }
}

pub(super) struct CgroupFsType;

impl FsType for CgroupFsType {
    fn name(&self) -> &'static str {
        "cgroup2"
    }

    fn properties(&self) -> FsProperties {
        FsProperties::empty()
    }

    fn create(
        &self,
        _flags: FsFlags,
        _args: Option<CString>,
        _disk: Option<Arc<dyn BlockDevice>>,
    ) -> Result<Arc<dyn FileSystem>> {
        Ok(CgroupFs::singleton().clone() as _)
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        Some(EmptyNode::new("cgroup".into()))
    }
}
