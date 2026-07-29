// SPDX-License-Identifier: MPL-2.0

//! Virtio-fs filesystem wiring.
//!
//! This module defines the filesystem type and objects for virtio-fs.

use aster_fuse::{
    EntryReply, FUSE_ROOT_ID, Kstatfs, StatfsOperation, ops::lookup::LookupOperation,
};
use aster_virtio::device::filesystem::device::{self, AttrVersion, FileSystemDevice, FuseSession};
use device_id::DeviceId;

use super::inode::{InodeCache, VirtioFsInode};
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
    inode_cache: InodeCache,
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

        Ok(Arc::new_cyclic(|weak_fs| {
            let root = VirtioFsInode::new_root(
                root_entry,
                weak_fs.clone(),
                container_dev_id,
                session.bump_attr_version(),
            );
            let inode_cache = InodeCache::new(&root);

            Self {
                sb,
                root,
                tag,
                session,
                inode_cache,
                fs_event_subscriber_stats: FsEventSubscriberStats::new(),
            }
        }))
    }

    pub(super) fn session(&self) -> &Arc<FuseSession> {
        &self.session
    }

    /// Returns the device ID of this virtio-fs mount.
    pub(super) fn container_device_id(&self) -> DeviceId {
        self.sb.container_dev_id
    }

    /// Reads an inode from a FUSE entry reply via the inode cache.
    pub(super) fn lookup_inode_from_cache(
        self: &Arc<Self>,
        lookup_reply: EntryReply,
        request_attr_version: AttrVersion,
    ) -> Result<Arc<VirtioFsInode>> {
        self.inode_cache
            .lookup_inode(lookup_reply, request_attr_version, self)
    }

    /// Inserts a newly created inode into the inode cache.
    pub(super) fn insert_inode_to_cache(&self, inode: &Arc<VirtioFsInode>) {
        self.inode_cache.insert_inode(inode);
    }

    /// Removes an inode from the cache if it is still the cached entry.
    pub(super) fn remove_inode_from_cache(
        &self,
        inode: &VirtioFsInode,
    ) -> Option<Weak<VirtioFsInode>> {
        self.inode_cache.remove_inode(inode)
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
