// SPDX-License-Identifier: MPL-2.0

//! Open directory handles for `virtiofs`.

use aster_fuse::FuseOpenFlags;

use super::{inode::VirtioFsInode, open_handle::VirtioFsOpenHandle};
use crate::{
    events::IoEvents,
    fs::{
        file::{PerOpenFileOps, StatusFlags},
        utils::DirentVisitor,
        vfs::inode::FileOps,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

/// A per-open directory object backed by a FUSE open handle.
///
/// Readdir and release requests carry this handle.
pub(super) struct VirtioFsDir {
    inode: Arc<VirtioFsInode>,
    open_handle: Arc<VirtioFsOpenHandle>,
}

impl VirtioFsDir {
    pub(super) fn new(inode: Arc<VirtioFsInode>, open_handle: Arc<VirtioFsOpenHandle>) -> Self {
        Self { inode, open_handle }
    }
}

impl Pollable for VirtioFsDir {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl FileOps for VirtioFsDir {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EISDIR, "the inode is a directory");
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EISDIR, "the inode is a directory");
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        self.inode.readdir(
            self.open_handle.fh(),
            offset,
            self.open_handle.file_flags(),
            visitor,
        )
    }
}

impl PerOpenFileOps for VirtioFsDir {
    fn check_seekable(&self) -> Result<()> {
        if self
            .open_handle
            .open_flags()
            .intersects(FuseOpenFlags::FOPEN_STREAM | FuseOpenFlags::FOPEN_NONSEEKABLE)
        {
            return_errno_with_message!(Errno::ESPIPE, "the directory is not seekable");
        }

        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        true
    }
}
