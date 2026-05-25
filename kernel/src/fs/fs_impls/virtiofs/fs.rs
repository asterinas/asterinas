// SPDX-License-Identifier: MPL-2.0

//! Virtio-fs filesystem wiring.
//!
//! This module defines the filesystem type and objects for virtio-fs.

use core::time::Duration;

use aster_fuse::{FUSE_ROOT_ID, Kstatfs, StatfsOperation, ops::lookup::LookupOperation};
use aster_virtio::device::filesystem::device::{self, FileSystemDevice, FuseSession};
use device_id::DeviceId;

use super::{inode::VirtioFsInode, valid_until};
use crate::{
    fs::{
        pseudofs::AnonDeviceId,
        utils::NAME_MAX,
        vfs::{
            file_system::{FileSystem, FsEventSubscriberStats, SuperBlock},
            inode::Inode,
            registry::{FsCreationCtx, FsProperties, FsType},
        },
    },
    prelude::*,
};

/// Filesystem magic reported for virtio-fs in `statfs`.
const VIRTIOFS_MAGIC: u64 = 0x6573_5546;

/// Block size reported to `statfs` for virtio-fs.
const BLOCK_SIZE: usize = 4096;

/// The `virtiofs` filesystem type.
pub(super) struct VirtioFsType;

impl FsType for VirtioFsType {
    fn name(&self) -> &'static str {
        "virtiofs"
    }

    fn properties(&self) -> FsProperties {
        FsProperties::empty()
    }

    fn create(&self, fs_creation_ctx: &FsCreationCtx) -> Result<Arc<dyn FileSystem>> {
        let tag = fs_creation_ctx
            .source()
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "virtiofs source(tag) is required"))?
            .to_string();

        let device = device::find_device_by_tag(&tag)
            .ok_or_else(|| Error::with_message(Errno::ENODEV, "virtiofs device is not found"))?;

        Ok(VirtioFs::new(device, tag)? as Arc<dyn FileSystem>)
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        None
    }
}

/// A mounted virtio-fs filesystem.
pub(super) struct VirtioFs {
    sb: SuperBlock,
    root: Arc<VirtioFsInode>,
    tag: String,
    session: Arc<FuseSession>,
    fs_event_subscriber_stats: FsEventSubscriberStats,
}

impl VirtioFs {
    fn new(device: Arc<FileSystemDevice>, tag: String) -> Result<Arc<Self>> {
        let session = FuseSession::new(device)
            .map_err(|_| Error::with_message(Errno::EIO, "virtiofs FUSE_INIT failed"))?;

        let anon_device_id =
            AnonDeviceId::acquire().expect("no device ID is available for virtiofs");
        let container_dev_id = anon_device_id.id();
        let statfs = session.do_fuse_op(FUSE_ROOT_ID, StatfsOperation)?.st();

        // TODO: Update the super block fields based on `statfs` reply.
        // For now, we set only the fields required by VFS when mounting the filesystem.
        // No update is made for these fields later.
        let sb = SuperBlock::from((container_dev_id, statfs));

        let root_entry = session.do_fuse_op(FUSE_ROOT_ID, LookupOperation::new("."))?;
        let root_metadata = super::inode::metadata_from_attr(root_entry.attr(), container_dev_id);
        let attr_valid_until = valid_until(root_entry.attr_valid(), root_entry.attr_valid_nsec());

        Ok(Arc::new_cyclic(|weak_fs| {
            let root = VirtioFsInode::new(
                FUSE_ROOT_ID,
                root_entry.generation(),
                root_metadata,
                weak_fs.clone(),
                Duration::MAX,
                attr_valid_until,
                session.bump_attr_version(),
            );

            Self {
                sb,
                root,
                tag,
                session,
                fs_event_subscriber_stats: FsEventSubscriberStats::new(),
            }
        }))
    }

    pub(super) fn session(&self) -> &Arc<FuseSession> {
        &self.session
    }
}

impl From<(DeviceId, Kstatfs)> for SuperBlock {
    fn from((container_dev_id, statfs): (DeviceId, Kstatfs)) -> Self {
        let mut sb = SuperBlock::new(VIRTIOFS_MAGIC, BLOCK_SIZE, NAME_MAX, container_dev_id);

        sb.blocks = statfs.blocks() as usize;
        sb.bfree = statfs.bfree() as usize;
        sb.bavail = statfs.bavail() as usize;
        sb.files = statfs.files() as usize;
        sb.ffree = statfs.ffree() as usize;
        sb.bsize = statfs.bsize() as usize;
        sb.namelen = statfs.namelen() as usize;
        sb.frsize = statfs.frsize() as usize;
        sb
    }
}

impl FileSystem for VirtioFs {
    fn name(&self) -> &'static str {
        "virtiofs"
    }

    fn source(&self) -> Option<&str> {
        Some(&self.tag)
    }

    // TODO: Implement `sync` by issuing `fsync` to open files and syncing the device if supported.
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
