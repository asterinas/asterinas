// SPDX-License-Identifier: MPL-2.0

//! Inode implementation for `virtiofs`.

mod metadata;
mod ops;
mod page_cache;

use core::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use aster_fuse::{
    DirentType, FuseGeneration, FuseNodeId, FuseOpenFlags, LookupCount, ReleaseFlags, ReleaseKind,
    SetattrReq, SetattrValid,
    ops::{
        link::{LinkOperation, LinkReq},
        lookup::LookupOperation,
        mkdir::{MkdirOperation, MkdirReq},
        mknod::{MknodOperation, MknodReq},
        open::{OpenReq, OpendirOperation},
        release::ReleaseOptions,
        rmdir::RmdirOperation,
        unlink::UnlinkOperation,
    },
};
use aster_virtio::device::filesystem::device::AttrVersion;
pub(super) use metadata::metadata_from_attr;

use super::{
    fs::VirtioFs,
    open_handle::{OpenHandles, VirtioFsOpenHandle},
    valid_until,
};
use crate::{
    fs::{
        file::{AccessMode, InodeMode, InodeType, PerOpenFileOps, StatusFlags},
        utils::DirentVisitor,
        vfs::{
            file_system::FileSystem,
            inode::{Extension, FileOps, Inode, Metadata, RevalidationPolicy, SymbolicLink},
        },
    },
    prelude::*,
    process::{Gid, Uid},
    vm::page_cache::PageCache,
};

/// Represents a cached virtio-fs inode and its kernel-side state.
pub(super) struct VirtioFsInode {
    nodeid: FuseNodeId,
    generation: FuseGeneration,
    type_: InodeType,
    lookup_count: LookupCount,
    /// The size of this inode.
    ///
    /// This field is intentionally kept outside `inner` to avoid deadlocks.
    /// Since `VirtioFsInode` serves as its own page cache backend, page cache
    /// operations may call back into this inode through the backend interface
    /// to query the EOF position (i.e., the inode size). If the size were stored
    /// inside `inner`, acquiring the `inner` lock would be required, which could
    /// lead to a deadlock given that the `inner` lock may already be held by the
    /// caller of the page cache operation.
    size: AtomicUsize,
    /// The metadata lock also serializes file data I/O for this inode.
    ///
    /// Lock order: `self.entry_valid_until` -> `self.inner`
    ///                 -> `open_handles.handles`
    ///
    /// `lookup_count` is not protected by any lock mentioned above and may be
    /// touched at any time.
    inner: RwMutex<InodeInner>,
    entry_valid_until: Mutex<Duration>,
    open_handles: OpenHandles,
    fs: Weak<VirtioFs>,
    extension: Extension,
    weak_self: Weak<Self>,
}

impl VirtioFsInode {
    /// Creates an inode from a fresh FUSE entry or attribute reply.
    pub(super) fn new(
        nodeid: FuseNodeId,
        generation: FuseGeneration,
        metadata: Metadata,
        fs: Weak<VirtioFs>,
        entry_valid_until: Duration,
        attr_valid_until: Duration,
        attr_version: AttrVersion,
    ) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            nodeid,
            generation,
            type_: metadata.type_,
            lookup_count: LookupCount::initial(),
            size: AtomicUsize::new(metadata.size),
            inner: RwMutex::new(InodeInner {
                page_cache: metadata.type_.is_regular_file().then(|| {
                    PageCache::new_with_backend(metadata.size, weak_self.clone() as _).unwrap()
                }),
                metadata,
                attr_valid_until,
                attr_version,
            }),
            entry_valid_until: Mutex::new(entry_valid_until),
            open_handles: OpenHandles::new(),
            fs,
            extension: Extension::new(),
            weak_self: weak_self.clone(),
        })
    }

    fn fs_ref(&self) -> Arc<VirtioFs> {
        self.fs.upgrade().unwrap()
    }

    pub(super) fn nodeid(&self) -> FuseNodeId {
        self.nodeid
    }

    pub(super) fn generation(&self) -> FuseGeneration {
        self.generation
    }

    pub(super) fn size(&self) -> usize {
        self.size.load(Ordering::Acquire)
    }

    fn type_(&self) -> InodeType {
        self.type_
    }
}

/// An inode timestamp field updated through `SETATTR`.
pub(super) enum TimeField {
    /// The last access timestamp.
    Access,
    /// The last metadata change timestamp.
    Change,
    /// The last content modification timestamp.
    Modify,
}

/// A write offset resolved while holding the inode inner lock.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WriteOffset {
    /// Writes at the caller-provided absolute offset.
    Absolute(usize),
    /// Writes at the current end of file.
    Append,
}

struct InodeInner {
    page_cache: Option<PageCache>,
    metadata: Metadata,
    attr_valid_until: Duration,
    attr_version: AttrVersion,
}

impl InodeInner {
    /// Invalidates the whole page cache, if any.
    fn invalidate_page_cache(&self) -> Result<()> {
        let Some(page_cache) = &self.page_cache else {
            return Ok(());
        };

        let cached_size = page_cache.size();
        if cached_size > 0 {
            page_cache.invalidate_range(0..cached_size)?;
        }

        Ok(())
    }

    /// Returns whether cached attributes are still inside the server TTL.
    fn is_attr_valid(&self, now: Duration) -> bool {
        now < self.attr_valid_until
    }

    /// Returns whether an incoming attribute reply can replace cached metadata.
    fn accepts_attr_version(&self, incoming: AttrVersion) -> bool {
        incoming >= self.attr_version
    }

    fn page_cache(&self) -> Option<&PageCache> {
        self.page_cache.as_ref()
    }
}

impl Inode for VirtioFsInode {
    fn size(&self) -> usize {
        self.size()
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        if self.type_() != InodeType::File {
            return_errno_with_message!(Errno::EISDIR, "resize on non-regular file");
        }

        let size = u64::try_from(new_size)
            .map_err(|_| Error::with_message(Errno::EFBIG, "virtiofs resize size too large"))?;

        let setattr_req = SetattrReq::new(SetattrValid::FATTR_SIZE).set_size(size);
        self.setattr(setattr_req)
    }

    fn metadata(&self) -> Metadata {
        self.inner.read().metadata
    }

    fn ino(&self) -> u64 {
        self.nodeid().as_u64()
    }

    fn type_(&self) -> InodeType {
        self.type_
    }

    fn mode(&self) -> Result<InodeMode> {
        Ok(self.inner.read().metadata.mode)
    }

    fn set_mode(&self, mode: InodeMode) -> Result<()> {
        let mode_bits = self.type_() as u32 | u32::from(mode.bits());
        let setattr_req = SetattrReq::new(SetattrValid::FATTR_MODE).set_mode(mode_bits);
        self.setattr(setattr_req)
    }

    fn owner(&self) -> Result<Uid> {
        Ok(self.inner.read().metadata.uid)
    }

    fn set_owner(&self, uid: Uid) -> Result<()> {
        let setattr_req = SetattrReq::new(SetattrValid::FATTR_UID).set_uid(uid.into());
        self.setattr(setattr_req)
    }

    fn group(&self) -> Result<Gid> {
        Ok(self.inner.read().metadata.gid)
    }

    fn set_group(&self, gid: Gid) -> Result<()> {
        let setattr_req = SetattrReq::new(SetattrValid::FATTR_GID).set_gid(gid.into());
        self.setattr(setattr_req)
    }

    fn atime(&self) -> Duration {
        self.inner.read().metadata.last_access_at
    }

    fn set_atime(&self, time: Duration) {
        self.set_time(TimeField::Access, time);
    }

    fn mtime(&self) -> Duration {
        self.inner.read().metadata.last_modify_at
    }

    fn set_mtime(&self, time: Duration) {
        self.set_time(TimeField::Modify, time);
    }

    fn ctime(&self) -> Duration {
        self.inner.read().metadata.last_meta_change_at
    }

    fn set_ctime(&self, time: Duration) {
        self.set_time(TimeField::Change, time);
    }

    fn page_cache(&self) -> Option<PageCache> {
        self.inner.read().page_cache.clone()
    }

    fn open(
        &self,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn PerOpenFileOps>>> {
        match self.type_ {
            InodeType::File => Some(self.open_file(access_mode, status_flags)),
            InodeType::Dir => Some(self.open_directory(access_mode, status_flags)),
            // TODO: Support opening special files like device files and named pipes.
            _ => Some(Err(Error::with_message(
                Errno::EOPNOTSUPP,
                "opening this virtiofs inode type is not supported",
            ))),
        }
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        let fs = self.fs_ref();
        let parent_nodeid = self.nodeid();
        let entry_reply = fs
            .session()
            .do_fuse_op(parent_nodeid, LookupOperation::new(name))?;
        let nodeid = entry_reply.nodeid();

        let entry_valid_until =
            valid_until(entry_reply.entry_valid(), entry_reply.entry_valid_nsec());
        let attr_valid_until = valid_until(entry_reply.attr_valid(), entry_reply.attr_valid_nsec());

        // TODO: Add an inode cache keyed by `(nodeid, generation)` so hard
        // links to the same FUSE inode share one `VirtioFsInode`.
        let inode = VirtioFsInode::new(
            nodeid,
            entry_reply.generation(),
            metadata_from_attr(entry_reply.attr(), fs.sb().container_dev_id),
            Arc::downgrade(&fs),
            entry_valid_until,
            attr_valid_until,
            fs.session().bump_attr_version(),
        );

        Ok(inode)
    }

    fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<dyn Inode>> {
        let fs = self.fs_ref();
        let parent_nodeid = self.nodeid();
        let entry_reply = match type_ {
            InodeType::File => fs.session().do_fuse_op(
                parent_nodeid,
                MknodOperation::new(
                    MknodReq::new(InodeType::File as u32 | u32::from(mode.bits()), 0),
                    name,
                ),
            )?,
            InodeType::Dir => fs.session().do_fuse_op(
                parent_nodeid,
                MkdirOperation::new(
                    MkdirReq::new(InodeType::Dir as u32 | u32::from(mode.bits())),
                    name,
                ),
            )?,
            InodeType::Socket => fs.session().do_fuse_op(
                parent_nodeid,
                MknodOperation::new(
                    MknodReq::new(InodeType::Socket as u32 | u32::from(mode.bits()), 0),
                    name,
                ),
            )?,
            _ => {
                return_errno_with_message!(
                    Errno::EOPNOTSUPP,
                    "virtiofs create supports file/dir/socket only"
                )
            }
        };
        Ok(VirtioFsInode::build_child_inode(&fs, entry_reply))
    }

    fn link(&self, old: &Arc<dyn Inode>, name: &str) -> Result<()> {
        let old = old
            .downcast_ref::<VirtioFsInode>()
            .ok_or_else(|| Error::with_message(Errno::EXDEV, "not same fs"))?;

        let fs = self.fs_ref();
        let request_attr_version = fs.session().snapshot_attr_version();
        let entry_reply = fs.session().do_fuse_op(
            self.nodeid(),
            LinkOperation::new(LinkReq::new(old.nodeid()), name),
        )?;
        old.commit_entry_reply(
            &entry_reply,
            request_attr_version,
            metadata::StaleAttrAction::MergeLink,
        )?;

        Ok(())
    }

    fn unlink(&self, name: &str) -> Result<()> {
        let fs = self.fs_ref();
        fs.session()
            .do_fuse_op(self.nodeid(), UnlinkOperation::new(name))?;
        Ok(())
    }

    fn rmdir(&self, name: &str) -> Result<()> {
        let fs = self.fs_ref();
        fs.session()
            .do_fuse_op(self.nodeid(), RmdirOperation::new(name))?;
        Ok(())
    }

    fn sync_data(&self) -> Result<()> {
        let inner = self.inner.write();
        let Some(page_cache) = &inner.page_cache else {
            return Ok(());
        };
        let cached_size = page_cache.size();
        if cached_size > 0 {
            page_cache.flush_range(0..cached_size)?;
        }

        Ok(())
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.fs_ref()
    }

    fn revalidation_policy(&self) -> RevalidationPolicy {
        match self.type_ {
            InodeType::Dir => {
                RevalidationPolicy::REVALIDATE_EXISTS | RevalidationPolicy::REVALIDATE_ABSENT
            }
            _ => RevalidationPolicy::empty(),
        }
    }

    fn revalidate_exists(&self, name: &str, child: &dyn Inode) -> bool {
        let Some(child) = child.downcast_ref::<VirtioFsInode>() else {
            return false;
        };

        child.revalidate_lookup(self.nodeid(), name).is_ok()
    }

    fn revalidate_absent(&self, _name: &str) -> bool {
        // FIXME: FUSE negative-entry caching is not implemented yet.
        // Force a fresh `FUSE_LOOKUP` for each negative lookup.
        false
    }

    fn read_link(&self) -> Result<SymbolicLink> {
        if self.type_ != InodeType::SymLink {
            return_errno_with_message!(Errno::EINVAL, "read_link on non-symlink")
        }

        let fs = self.fs_ref();
        let target = fs.session().readlink(self.nodeid())?;

        Ok(SymbolicLink::Plain(target))
    }

    fn extension(&self) -> &Extension {
        &self.extension
    }
}

impl FileOps for VirtioFsInode {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        if self.type_ != InodeType::File {
            return_errno_with_message!(
                Errno::EBADF,
                "virtiofs inode I/O requires an open file handle"
            );
        }

        // `execve` may call `read_at` on the inode directly, bypassing the
        // normal file-open path. Open a transient handle so FUSE I/O still has
        // a server-provided file handle when this inode has no cached open.
        let handle = self.open_transient_handle(AccessMode::O_RDONLY)?;
        self.direct_read_at(offset, writer, handle.fh(), handle.file_flags())
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        // No current path should call inode-level `write_at` for virtio-fs:
        // normal writes go through the per-open `VirtioFsFile`, which owns the
        // server-provided handle from `FUSE_OPEN`. If a future direct-inode
        // writer appears, it must define the handle semantics explicitly.
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "virtiofs inode write_at without an open file handle is not supported"
        )
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        let fs = self.fs_ref();
        let open_out = fs
            .session()
            .do_fuse_op(self.nodeid(), OpendirOperation::new(OpenReq::new(0)))?;

        let dir_handle = VirtioFsOpenHandle::new(
            open_out.fh(),
            self.nodeid(),
            AccessMode::O_RDONLY,
            StatusFlags::empty(),
            open_out.open_flags(),
            self.fs.clone(),
            ReleaseOptions::new(ReleaseKind::Directory, ReleaseFlags::empty()),
        );

        if !dir_handle
            .open_flags()
            .contains(FuseOpenFlags::FOPEN_KEEP_CACHE)
        {
            self.invalidate_whole_page_cache()?;
        }

        // FIXME: `readdir_at` exposes the delta-based interface, while
        // FUSE readdir offsets are opaque continuation cookies.
        self.readdir(dir_handle.fh(), offset, dir_handle.file_flags(), visitor)
    }
}

impl From<DirentType> for InodeType {
    fn from(type_: DirentType) -> Self {
        match type_ {
            DirentType::Dir => InodeType::Dir,
            DirentType::Regular => InodeType::File,
            DirentType::Link => InodeType::SymLink,
            DirentType::Char => InodeType::CharDevice,
            DirentType::Block => InodeType::BlockDevice,
            DirentType::Fifo => InodeType::NamedPipe,
            DirentType::Sock => InodeType::Socket,
            DirentType::Unknown => InodeType::Unknown,
        }
    }
}
