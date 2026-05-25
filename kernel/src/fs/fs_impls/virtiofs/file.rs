// SPDX-License-Identifier: MPL-2.0

//! Open regular-file handles for `virtiofs`.

use aster_fuse::FuseOpenFlags;
use ostd::warn;

use super::{
    inode::{VirtioFsInode, WriteOffset},
    open_handle::VirtioFsOpenHandle,
};
use crate::{
    events::IoEvents,
    fs::{
        file::{PerOpenFileOps, StatusFlags},
        vfs::inode::FileOps,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    thread::work_queue::{self, WorkPriority},
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
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        let fh = self.open_handle.fh();
        let file_flags = self.open_handle.file_flags();

        match self.cache_policy {
            CachePolicy::Cached => self.inode.cached_read_at(offset, writer, fh, file_flags),
            CachePolicy::Direct => self.inode.direct_read_at(offset, writer, fh, file_flags),
        }
    }

    fn write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        let write_offset = if status_flags.contains(StatusFlags::O_APPEND) {
            self.inode.revalidate_attr(self.open_handle.fh())?;
            WriteOffset::Append
        } else {
            WriteOffset::Absolute(offset)
        };

        // FIXME: Cached writeback currently submits whole-page writes with the
        // original open flags. With `O_APPEND`, the server may append cached
        // bytes that precede the user write. Keep append writes on the direct
        // path until writeback can issue precise positional ranges without
        // append semantics.
        if self.cache_policy == CachePolicy::Cached && !status_flags.contains(StatusFlags::O_APPEND)
        {
            self.inode.cached_write_at(
                write_offset,
                reader,
                self.open_handle.fh(),
                self.open_handle.file_flags(),
            )
        } else {
            self.inode.direct_write_at(
                write_offset,
                reader,
                self.open_handle.fh(),
                self.open_handle.file_flags(),
            )
        }
    }
}

impl PerOpenFileOps for VirtioFsFile {
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
        self.inode.revalidate_attr(self.open_handle.fh())?;

        Ok(Some(self.inode.size()))
    }
}

/// The virtio-fs file I/O caching policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CachePolicy {
    /// I/O goes through the page cache.
    Cached,
    /// I/O bypasses the page cache and hits the FUSE server directly.
    Direct,
}
