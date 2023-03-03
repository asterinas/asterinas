//! Opend Inode-backed File Handle

mod dyn_cap;
mod static_cap;

use crate::fs::utils::{
    AccessMode, Dentry, DirentWriter, DirentWriterContext, InodeType, SeekFrom, StatusFlags,
};
use crate::prelude::*;
use crate::rights::Rights;

pub struct InodeHandle<R = Rights>(Arc<InodeHandle_>, R);

struct InodeHandle_ {
    dentry: Arc<Dentry>,
    offset: Mutex<usize>,
    access_mode: AccessMode,
    status_flags: Mutex<StatusFlags>,
}

impl InodeHandle_ {
    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        let mut offset = self.offset.lock();
        let len = if self.status_flags.lock().contains(StatusFlags::O_DIRECT) {
            self.dentry.vnode().read_direct_at(*offset, buf)?
        } else {
            self.dentry.vnode().read_at(*offset, buf)?
        };

        *offset += len;
        Ok(len)
    }

    pub fn write(&self, buf: &[u8]) -> Result<usize> {
        let mut offset = self.offset.lock();
        if self.status_flags.lock().contains(StatusFlags::O_APPEND) {
            *offset = self.dentry.vnode().len();
        }
        let len = if self.status_flags.lock().contains(StatusFlags::O_DIRECT) {
            self.dentry.vnode().write_direct_at(*offset, buf)?
        } else {
            self.dentry.vnode().write_at(*offset, buf)?
        };

        *offset += len;
        Ok(len)
    }

    pub fn read_to_end(&self, buf: &mut Vec<u8>) -> Result<usize> {
        let len = if self.status_flags.lock().contains(StatusFlags::O_DIRECT) {
            self.dentry.vnode().read_direct_to_end(buf)?
        } else {
            self.dentry.vnode().read_to_end(buf)?
        };
        Ok(len)
    }

    pub fn seek(&self, pos: SeekFrom) -> Result<usize> {
        let mut offset = self.offset.lock();
        let new_offset: isize = match pos {
            SeekFrom::Start(off /* as usize */) => {
                if off > isize::max_value() as usize {
                    return_errno_with_message!(Errno::EINVAL, "file offset is too large");
                }
                off as isize
            }
            SeekFrom::End(off /* as isize */) => {
                let file_size = self.dentry.vnode().len() as isize;
                assert!(file_size >= 0);
                file_size
                    .checked_add(off)
                    .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "file offset overflow"))?
            }
            SeekFrom::Current(off /* as isize */) => (*offset as isize)
                .checked_add(off)
                .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "file offset overflow"))?,
        };
        if new_offset < 0 {
            return_errno_with_message!(Errno::EINVAL, "file offset must not be negative");
        }
        // Invariant: 0 <= new_offset <= isize::max_value()
        let new_offset = new_offset as usize;
        *offset = new_offset;
        Ok(new_offset)
    }

    pub fn offset(&self) -> usize {
        let offset = self.offset.lock();
        *offset
    }

    pub fn len(&self) -> usize {
        self.dentry.vnode().len()
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
        let written_size = self.dentry.vnode().readdir(&mut dir_writer_ctx)?;
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

    pub fn len(&self) -> usize {
        self.0.len()
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

    pub fn dentry(&self) -> &Arc<Dentry> {
        &self.0.dentry
    }
}
