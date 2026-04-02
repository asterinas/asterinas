// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use ostd::mm::{VmReader, VmWriter};

use super::inode::VirtioFsInode;
use crate::{
    events::IoEvents,
    fs::{
        file::{FileIo, SeekFrom, StatusFlags},
        vfs::inode::InodeIo,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    thread::work_queue::{WorkPriority, submit_work_func},
};

pub(super) struct VirtioFsHandle {
    inode: Arc<VirtioFsInode>,
    fh: u64,
    flags: u32,
    cache_enabled: bool,
}

impl VirtioFsHandle {
    pub(super) fn new(inode: Arc<VirtioFsInode>, fh: u64, flags: u32, cache_enabled: bool) -> Self {
        Self {
            inode,
            fh,
            flags,
            cache_enabled,
        }
    }
}

impl Drop for VirtioFsHandle {
    fn drop(&mut self) {
        let inode = self.inode.clone();
        let fh = self.fh;
        let flags = self.flags;
        let cache_enabled = self.cache_enabled;

        submit_work_func(
            move || {
                if cache_enabled {
                    let _ = inode.flush_page_cache();
                }

                if let Some(fs) = inode.try_fs_ref() {
                    let _ = fs.device.fuse_release(inode.nodeid(), fh, flags);
                }
            },
            WorkPriority::Normal,
        );
    }
}

impl Pollable for VirtioFsHandle {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl InodeIo for VirtioFsHandle {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        if self.cache_enabled {
            self.inode.cached_read_at(offset, writer, self.fh)
        } else {
            self.inode.direct_read_at(offset, writer, self.fh)
        }
    }

    fn write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        let offset = if status_flags.contains(StatusFlags::O_APPEND) {
            self.inode.revalidate_attr()?;
            self.inode.size()
        } else {
            offset
        };

        if self.cache_enabled {
            self.inode.cached_write_at(offset, reader, self.fh)
        } else {
            self.inode.direct_write_at(offset, reader, self.fh)
        }
    }
}

impl FileIo for VirtioFsHandle {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        true
    }

    fn seek(&self, offset: &Mutex<usize>, pos: SeekFrom) -> Result<usize> {
        let end = match pos {
            SeekFrom::End(_) => {
                self.inode.revalidate_attr()?;
                Some(self.inode.size())
            }
            _ => None,
        };

        let mut offset = offset.lock();
        let new_offset = match pos {
            SeekFrom::Start(off) => off,
            SeekFrom::Current(diff) => offset.wrapping_add_signed(diff),
            SeekFrom::End(diff) => end.unwrap().wrapping_add_signed(diff),
        };

        if new_offset.cast_signed() < 0 {
            return_errno_with_message!(Errno::EINVAL, "the file offset cannot be negative");
        }

        *offset = new_offset;
        Ok(new_offset)
    }
}
