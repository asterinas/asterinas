// SPDX-License-Identifier: MPL-2.0

//! Metadata cache and attribute updates for `VirtioFsInode`.

use core::{sync::atomic::Ordering, time::Duration};

use aster_fuse::{Attr, FuseAttrReply, FuseInitFlags, SetattrValid};
use aster_virtio::device::filesystem::device::AttrVersion;
use device_id::DeviceId;

use super::{super::valid_until, InodeInner, VirtioFsInode};
use crate::{
    fs::{
        file::{InodeMode, InodeType},
        vfs::{file_system::FileSystem, inode::Metadata},
    },
    prelude::*,
    process::{Gid, Uid},
    time::clocks::{MonotonicCoarseClock, RealTimeCoarseClock},
};

impl VirtioFsInode {
    /// Commits a FUSE attribute reply into the inode metadata cache.
    ///
    /// If `request_attr_version` is still accepted by the inode, the reply is
    /// installed as a complete metadata snapshot and its TTL becomes the new
    /// attribute-cache deadline. If the reply is stale, `stale_action` decides
    /// whether the reply is ignored or whether selected fields may still be
    /// merged. Page-cache invalidation is derived from the committed metadata
    /// change and is performed after dropping the metadata lock.
    pub(super) fn commit_attr_reply(
        &self,
        attr_reply: FuseAttrReply,
        request_attr_version: AttrVersion,
        stale_action: StaleAttrAction,
    ) -> Result<()> {
        let fs = self.fs_ref();
        let now = MonotonicCoarseClock::get().read_time();
        let metadata = metadata_from_attr(attr_reply.attr(), fs.sb().container_dev_id);
        let attr_valid_until = valid_until(attr_reply.attr_valid(), attr_reply.attr_valid_nsec());
        let session_flags = fs.session().negotiated_flags();

        let mut inner = self.inner.write();
        let old_size = self.size();
        let old_mtime = inner.metadata.last_modify_at;

        if inner.accepts_attr_version(request_attr_version) {
            let attr_version = fs.session().bump_attr_version();
            inner.commit_attr_snapshot(metadata, attr_valid_until, attr_version);
        } else {
            match stale_action {
                StaleAttrAction::Discard => {}
                StaleAttrAction::MergeSetattr(valid) => {
                    let attr_version = fs.session().bump_attr_version();
                    inner.merge_stale_setattr(metadata, valid, now, attr_version);
                }
                StaleAttrAction::MergeLink => {
                    let attr_version = fs.session().bump_attr_version();
                    inner.merge_stale_link(metadata, now, attr_version);
                }
            }
        }

        let new_size = inner.metadata.size;
        self.set_size(new_size);
        let is_mtime_changed = old_mtime != inner.metadata.last_modify_at;

        let Some(page_cache) = &inner.page_cache else {
            return Ok(());
        };

        if new_size != old_size {
            page_cache.resize(new_size, old_size)?;
        }

        if session_flags.contains(FuseInitFlags::AUTO_INVAL_DATA) && is_mtime_changed {
            inner.invalidate_page_cache()?;
        }

        Ok(())
    }

    /// Expires cached attributes without changing cached metadata.
    ///
    /// Calls this after operations that make the cached attributes potentially
    /// incomplete but do not return a full attribute reply for this inode.
    pub(super) fn expire_attr_cache(&self) {
        self.inner.write().attr_valid_until = MonotonicCoarseClock::get().read_time();
    }

    pub(super) fn set_size(&self, size: usize) {
        self.size.store(size, Ordering::Release);
    }
}

impl InodeInner {
    /// Commits metadata changes after a local write.
    pub(super) fn commit_local_write(&mut self, committed_size: usize, attr_version: AttrVersion) {
        let now = RealTimeCoarseClock::get().read_time();
        self.metadata.size = committed_size;
        self.metadata.nr_sectors_allocated = committed_size.div_ceil(512);
        self.metadata.last_modify_at = now;
        self.metadata.last_meta_change_at = now;
        self.attr_valid_until = MonotonicCoarseClock::get().read_time();
        self.attr_version = attr_version;
    }

    /// Commits a version-accepted attribute reply as complete cached metadata.
    fn commit_attr_snapshot(
        &mut self,
        metadata: Metadata,
        attr_valid_until: Duration,
        attr_version: AttrVersion,
    ) {
        self.metadata = metadata;
        self.attr_valid_until = attr_valid_until;
        self.attr_version = attr_version;
    }

    /// Merges fields changed by a stale successful `SETATTR` reply.
    fn merge_stale_setattr(
        &mut self,
        metadata: Metadata,
        valid: SetattrValid,
        now: Duration,
        attr_version: AttrVersion,
    ) {
        self.merge_setattr_fields(metadata, valid);
        self.attr_valid_until = now;
        self.attr_version = attr_version;
    }

    /// Merges fields changed by a stale successful `LINK` reply.
    fn merge_stale_link(&mut self, metadata: Metadata, now: Duration, attr_version: AttrVersion) {
        self.metadata.nr_hard_links = metadata.nr_hard_links;
        self.metadata.last_meta_change_at = metadata.last_meta_change_at;
        self.attr_valid_until = now;
        self.attr_version = attr_version;
    }

    /// Merges fields selected by a `SETATTR` valid mask.
    fn merge_setattr_fields(&mut self, metadata: Metadata, valid: SetattrValid) {
        if valid.contains(SetattrValid::FATTR_MODE) {
            self.metadata.type_ = metadata.type_;
            self.metadata.mode = metadata.mode;
        }
        if valid.contains(SetattrValid::FATTR_UID) {
            self.metadata.uid = metadata.uid;
        }
        if valid.contains(SetattrValid::FATTR_GID) {
            self.metadata.gid = metadata.gid;
        }
        if valid.contains(SetattrValid::FATTR_SIZE) {
            self.metadata.size = metadata.size;
            self.metadata.nr_sectors_allocated = metadata.nr_sectors_allocated;
        }
        if valid.intersects(SetattrValid::FATTR_ATIME | SetattrValid::FATTR_ATIME_NOW) {
            self.metadata.last_access_at = metadata.last_access_at;
        }
        if valid.intersects(SetattrValid::FATTR_MTIME | SetattrValid::FATTR_MTIME_NOW) {
            self.metadata.last_modify_at = metadata.last_modify_at;
        }
        if valid.intersects(
            SetattrValid::FATTR_MODE
                | SetattrValid::FATTR_UID
                | SetattrValid::FATTR_GID
                | SetattrValid::FATTR_SIZE
                | SetattrValid::FATTR_ATIME
                | SetattrValid::FATTR_MTIME
                | SetattrValid::FATTR_ATIME_NOW
                | SetattrValid::FATTR_MTIME_NOW
                | SetattrValid::FATTR_CTIME,
        ) {
            self.metadata.last_meta_change_at = metadata.last_meta_change_at;
        }
    }
}

/// Describes what remains safe to commit when an attribute reply is stale.
///
/// Fresh replies always replace the full cached metadata. This enum is only
/// consulted after another local metadata commit has already advanced the
/// inode's `AttrVersion` past the request snapshot.
#[derive(Clone, Copy)]
pub(super) enum StaleAttrAction {
    /// Discards the stale reply without changing cached metadata or TTL.
    Discard,
    /// Merges fields changed by a successful `SETATTR` request.
    MergeSetattr(SetattrValid),
    /// Merges fields changed by a successful `LINK` request.
    MergeLink,
}

/// Converts a FUSE `Attr` into the VFS `Metadata` structure.
pub(in crate::fs::fs_impls::virtiofs) fn metadata_from_attr(
    attr: Attr,
    container_dev_id: DeviceId,
) -> Metadata {
    Metadata {
        ino: attr.ino(),
        size: attr.size() as usize,
        optimal_block_size: attr.blksize() as usize,
        nr_sectors_allocated: attr.blocks() as usize,
        last_access_at: Duration::new(attr.atime(), attr.atimensec()),
        last_modify_at: Duration::new(attr.mtime(), attr.mtimensec()),
        last_meta_change_at: Duration::new(attr.ctime(), attr.ctimensec()),
        type_: InodeType::from_raw_mode(attr.mode() as u16).unwrap_or(InodeType::Unknown),
        mode: InodeMode::from_bits_truncate(attr.mode() as u16),
        nr_hard_links: attr.nlink() as usize,
        uid: Uid::new(attr.uid()),
        gid: Gid::new(attr.gid()),
        container_dev_id,
        self_dev_id: if attr.rdev() == 0 {
            None
        } else {
            DeviceId::from_encoded_u64(attr.rdev() as u64)
        },
        birth_at: None,
    }
}
