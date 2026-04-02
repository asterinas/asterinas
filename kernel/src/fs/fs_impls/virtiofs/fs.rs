// SPDX-License-Identifier: MPL-2.0

mod handle;
mod inode;

use alloc::{
    string::{String, ToString},
    sync::Arc,
};
use core::time::Duration;

use aster_virtio::device::filesystem::{
    device::{FileSystemDevice, get_device_by_tag},
    protocol::FUSE_ROOT_ID,
};
use device_id::DeviceId;

use self::inode::VirtioFsInode;
use crate::{
    fs::{
        utils::NAME_MAX,
        vfs::{
            file_system::{FileSystem, FsEventSubscriberStats, FsFlags, SuperBlock},
            inode::{Inode, Metadata},
            registry::{FsProperties, FsType},
        },
    },
    prelude::*,
    time::clocks::MonotonicCoarseClock,
};

const VIRTIOFS_MAGIC: u64 = 0x6573_5546;
const BLOCK_SIZE: usize = 4096;
pub(super) const FUSE_READDIR_BUF_SIZE: u32 = 4096;

pub(super) struct VirtioFsType;

impl FsType for VirtioFsType {
    fn name(&self) -> &'static str {
        "virtiofs"
    }

    fn properties(&self) -> FsProperties {
        FsProperties::empty()
    }

    fn create(
        &self,
        _flags: FsFlags,
        args: Option<CString>,
        _disk: Option<Arc<dyn aster_block::BlockDevice>>,
    ) -> Result<Arc<dyn FileSystem>> {
        let tag = args
            .as_ref()
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "virtiofs source(tag) is required"))?
            .to_str()
            .map_err(|_| Error::with_message(Errno::EINVAL, "invalid virtiofs tag"))?
            .to_string();

        let device = get_device_by_tag(&tag)
            .ok_or_else(|| Error::with_message(Errno::ENODEV, "virtiofs device tag not found"))?;

        Ok(VirtioFs::new(device, tag)? as _)
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        None
    }
}

pub(super) struct VirtioFs {
    sb: SuperBlock,
    root: Arc<VirtioFsInode>,
    tag: String,
    pub(super) device: Arc<FileSystemDevice>,
    fs_event_subscriber_stats: FsEventSubscriberStats,
}

impl VirtioFs {
    fn new(device: Arc<FileSystemDevice>, tag: String) -> Result<Arc<Self>> {
        let fuse_attr_out = device.fuse_getattr(FUSE_ROOT_ID).map_err(Error::from)?;
        let root_metadata = Metadata::from(fuse_attr_out.attr);

        let attr_valid_until = {
            let now = MonotonicCoarseClock::get().read_time();
            now.saturating_add(valid_duration(
                fuse_attr_out.attr_valid,
                fuse_attr_out.attr_valid_nsec,
            ))
        };

        Ok(Arc::new_cyclic(|weak_fs| {
            let root = VirtioFsInode::new(
                FUSE_ROOT_ID,
                root_metadata,
                weak_fs.clone(),
                None,
                attr_valid_until,
            );

            Self {
                sb: SuperBlock::new(VIRTIOFS_MAGIC, BLOCK_SIZE, NAME_MAX, DeviceId::null()),
                root,
                tag,
                device,
                fs_event_subscriber_stats: FsEventSubscriberStats::new(),
            }
        }))
    }
}

impl FileSystem for VirtioFs {
    fn name(&self) -> &'static str {
        "virtiofs"
    }

    fn source(&self) -> Option<&str> {
        Some(&self.tag)
    }

    // lxh TODO: implement sync by issuing fsync to all open files and sync to the device if supported
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

pub(super) fn valid_duration(secs: u64, nsecs: u32) -> Duration {
    let extra_secs = (nsecs / 1_000_000_000) as u64;
    let nanos = (nsecs % 1_000_000_000) as u64;
    Duration::from_secs(secs.saturating_add(extra_secs)).saturating_add(Duration::from_nanos(nanos))
}
