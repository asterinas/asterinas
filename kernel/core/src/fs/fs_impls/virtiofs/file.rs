// SPDX-License-Identifier: MPL-2.0

//! Open regular-file handles for `virtiofs`.

use aster_fuse::{FuseOpenFlags, ops::init::FuseInitFlags2};
use ostd::warn;

use super::{
    inode::{VirtioFsInode, WriteOffset},
    open_handle::VirtioFsOpenHandle,
};
use crate::{
    events::IoEvents,
    fs::{
        file::{Mappable, PerOpenFileOps, StatusFlags},
        vfs::inode::{FileOps, Inode},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    thread::work_queue::{self, WorkPriority},
    vm::vmar::FileMmapRequest,
};

/// A per-open file object backed by a FUSE file handle.
///
/// Each instance owns one server-issued `fh` returned by `FUSE_OPEN`. Read,
/// write, seek, and release requests carry this handle, while access rights
/// are inherited from the VFS open path that created the object.
///
/// The handle also records whether I/O should use the page cache or bypass it,
/// according to the flags returned by the server.
pub(super) struct VirtioFsFile {
    inode: Arc<VirtioFsInode>,
    open_handle: Arc<VirtioFsOpenHandle>,
    cache_policy: CachePolicy,
}

impl VirtioFsFile {
    pub(super) fn new(
        inode: Arc<VirtioFsInode>,
        open_handle: Arc<VirtioFsOpenHandle>,
        cache_policy: CachePolicy,
    ) -> Self {
        Self {
            inode,
            open_handle,
            cache_policy,
        }
    }
}

impl Drop for VirtioFsFile {
    fn drop(&mut self) {
        if self.cache_policy != CachePolicy::Cached {
            return;
        }

        let inode = self.inode.clone();
        let open_handle = self.open_handle.clone();

        work_queue::submit_work_func(
            move || {
                if let Err(err) = inode.invalidate_whole_page_cache() {
                    warn!(
                        "virtiofs flush before release failed for inode {:?}: {:?}",
                        inode.nodeid(),
                        err
                    );
                }

                // Keep the handle alive until invalidation finishes, so
                // `VirtioFsOpenHandle::drop` submits `FUSE_RELEASE` afterward.
                let _ = &open_handle;
            },
            WorkPriority::Normal,
        );
    }
}

impl Pollable for VirtioFsFile {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl FileOps for VirtioFsFile {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        let fh = self.open_handle.fh();
        let file_flags = self.open_handle.access_mode() as u32 | status_flags.bits();

        if self.cache_policy == CachePolicy::Cached && !status_flags.contains(StatusFlags::O_DIRECT)
        {
            self.inode.cached_read_at(offset, writer, fh, file_flags)
        } else {
            self.inode.direct_read_at(offset, writer, fh, file_flags)
        }
    }

    fn write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        let fh = self.open_handle.fh();
        let file_flags = self.open_handle.access_mode() as u32 | status_flags.bits();

        let write_offset = if status_flags.contains(StatusFlags::O_APPEND) {
            self.inode.revalidate_attr(Some(fh))?;
            WriteOffset::Append
        } else {
            WriteOffset::Absolute(offset)
        };

        // FIXME: Cached writeback currently submits whole-page writes with the
        // original open flags. With `O_APPEND`, the server may append cached
        // bytes that precede the user write. Keep append writes on the direct
        // path until writeback can issue precise positional ranges without
        // append semantics.
        if self.cache_policy == CachePolicy::Cached
            && !status_flags.intersects(StatusFlags::O_APPEND | StatusFlags::O_DIRECT)
        {
            self.inode
                .cached_write_at(write_offset, reader, fh, file_flags)
        } else {
            self.inode
                .direct_write_at(write_offset, reader, fh, file_flags)
        }
    }
}

impl PerOpenFileOps for VirtioFsFile {
    fn mappable(&self, request: FileMmapRequest) -> Result<Mappable> {
        // `FuseOpenFlags::FOPEN_DIRECT_IO` permits private mappings. Shared
        // mappings additionally require `FuseInitFlags2::DIRECT_IO_ALLOW_MMAP`
        // to have been negotiated.
        let is_mmap_allowed = !self
            .open_handle
            .open_flags()
            .contains(FuseOpenFlags::FOPEN_DIRECT_IO)
            || !request.is_shared()
            || self
                .inode
                .fs_ref()
                .session()
                .negotiated_flags2()
                .contains(FuseInitFlags2::DIRECT_IO_ALLOW_MMAP);

        if !is_mmap_allowed {
            return_errno_with_message!(Errno::ENODEV, "the file is not mappable");
        }

        self.inode
            .page_cache()
            .map(Mappable::Vmo)
            .ok_or_else(|| Error::with_message(Errno::ENODEV, "the file is not mappable"))
    }

    fn check_seekable(&self) -> Result<()> {
        if self
            .open_handle
            .open_flags()
            .intersects(FuseOpenFlags::FOPEN_STREAM | FuseOpenFlags::FOPEN_NONSEEKABLE)
        {
            return_errno_with_message!(Errno::ESPIPE, "the file is not seekable");
        }
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        true
    }

    fn seek_end(&self) -> Result<Option<usize>> {
        // The cached inode size may be stale. Refreshing attributes here keeps
        // `SEEK_END` consistent with the latest file size on the server.
        self.inode.revalidate_attr(Some(self.open_handle.fh()))?;

        Ok(Some(self.inode.size()))
    }
}

/// The virtio-fs file I/O caching policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CachePolicy {
    /// I/O goes through the page cache.
    Cached,
    /// I/O bypasses the page cache and hits the FUSE server directly, due to
    /// either `O_DIRECT` or `FOPEN_DIRECT_IO`.
    Direct,
}
