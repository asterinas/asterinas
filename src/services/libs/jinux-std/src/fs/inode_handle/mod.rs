//! Opend File Handle

mod dyn_cap;
mod static_cap;

use super::utils::{
    AccessMode, DirentWriter, DirentWriterContext, Inode, InodeType, SeekFrom, StatusFlags,
};
use crate::prelude::*;
use crate::rights::Rights;
use alloc::sync::Arc;

pub struct InodeHandle<R = Rights>(Arc<InodeHandle_>, R);

struct InodeHandle_ {
    inode: Arc<dyn Inode>,
    offset: Mutex<usize>,
    access_mode: AccessMode,
    status_flags: Mutex<StatusFlags>,
}

impl InodeHandle_ {
    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        let mut offset = self.offset.lock();
        let file_size = self.inode.metadata().size;
        let start = file_size.min(*offset);
        let end = file_size.min(*offset + buf.len());
        let len = if self.status_flags.lock().contains(StatusFlags::O_DIRECT) {
            self.inode.read_at(start, &mut buf[0..start - end])?
        } else {
            self.inode.read_at(start, &mut buf[0..start - end])?
            // TODO: use page cache
            // self.inode.pages().read_at(start, buf[0..start - end])?
        };

        *offset += len;
        Ok(len)
    }

    pub fn write(&self, buf: &[u8]) -> Result<usize> {
        let mut offset = self.offset.lock();
        let file_size = self.inode.metadata().size;
        if self.status_flags.lock().contains(StatusFlags::O_APPEND) {
            *offset = file_size;
        }
        let len = if self.status_flags.lock().contains(StatusFlags::O_DIRECT) {
            self.inode.write_at(*offset, buf)?
        } else {
            self.inode.write_at(*offset, buf)?
            // TODO: use page cache
            // let len = self.inode.pages().write_at(*offset, buf)?;
            // if offset + len > file_size {
            //     self.inode.resize(offset + len)?;
            // }
            // len
        };

        *offset += len;
        Ok(len)
    }

    pub fn seek(&self, pos: SeekFrom) -> Result<usize> {
        let mut offset = self.offset.lock();
        let new_offset: i64 = match pos {
            SeekFrom::Start(off /* as u64 */) => {
                if off > i64::max_value() as u64 {
                    return_errno_with_message!(Errno::EINVAL, "file offset is too large");
                }
                off as i64
            }
            SeekFrom::End(off /* as i64 */) => {
                let file_size = self.inode.metadata().size as i64;
                assert!(file_size >= 0);
                file_size
                    .checked_add(off)
                    .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "file offset overflow"))?
            }
            SeekFrom::Current(off /* as i64 */) => (*offset as i64)
                .checked_add(off)
                .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "file offset overflow"))?,
        };
        if new_offset < 0 {
            return_errno_with_message!(Errno::EINVAL, "file offset must not be negative");
        }
        // Invariant: 0 <= new_offset <= i64::max_value()
        let new_offset = new_offset as usize;
        *offset = new_offset;
        Ok(new_offset)
    }

    pub fn offset(&self) -> usize {
        let offset = self.offset.lock();
        *offset
    }

    pub fn access_mode(&self) -> AccessMode {
        self.access_mode
    }

    pub fn status_flags(&self) -> StatusFlags {
        let status_flags = self.status_flags.lock();
        *status_flags
    }

    pub fn set_status_flags(&self, new_status_flags: StatusFlags) {
        let mut status_flags = self.status_flags.lock();
        // Can change only the O_APPEND, O_ASYNC, O_NOATIME, and O_NONBLOCK flags
        let valid_flags_mask = StatusFlags::O_APPEND
            | StatusFlags::O_ASYNC
            | StatusFlags::O_NOATIME
            | StatusFlags::O_NONBLOCK;
        status_flags.remove(valid_flags_mask);
        status_flags.insert(new_status_flags & valid_flags_mask);
    }

    pub fn readdir(&self, writer: &mut dyn DirentWriter) -> Result<usize> {
        let mut offset = self.offset.lock();
        let mut dir_writer_ctx = DirentWriterContext::new(*offset, writer);
        let written_size = self.inode.readdir(&mut dir_writer_ctx)?;
        *offset = dir_writer_ctx.pos();
        Ok(written_size)
    }
}

/// Methods for both dyn and static
impl<R> InodeHandle<R> {
    pub fn seek(&self, pos: SeekFrom) -> Result<usize> {
        self.0.seek(pos)
    }

    pub fn offset(&self) -> usize {
        self.0.offset()
    }

    pub fn access_mode(&self) -> AccessMode {
        self.0.access_mode()
    }

    pub fn status_flags(&self) -> StatusFlags {
        self.0.status_flags()
    }

    pub fn set_status_flags(&self, new_status_flags: StatusFlags) {
        self.0.set_status_flags(new_status_flags)
    }
}
