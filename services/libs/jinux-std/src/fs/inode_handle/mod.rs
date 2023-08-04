//! Opend Inode-backed File Handle

mod dyn_cap;
mod static_cap;

use core::sync::atomic::{AtomicU32, Ordering};

use crate::fs::file_handle::FileLike;
use crate::fs::utils::{
    AccessMode, Dentry, DirentVisitor, InodeType, IoEvents, IoctlCmd, Metadata, Poller, SeekFrom,
    StatusFlags,
};
use crate::prelude::*;
use jinux_rights::Rights;

#[derive(Debug)]
pub struct InodeHandle<R = Rights>(Arc<InodeHandle_>, R);

struct InodeHandle_ {
    dentry: Arc<Dentry>,
    offset: Mutex<usize>,
    access_mode: AccessMode,
    status_flags: AtomicU32,
}

impl InodeHandle_ {
    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        let mut offset = self.offset.lock();
        let len = if self.status_flags().contains(StatusFlags::O_DIRECT) {
            self.dentry.vnode().read_direct_at(*offset, buf)?
        } else {
            self.dentry.vnode().read_at(*offset, buf)?
        };

        *offset += len;
        Ok(len)
    }

    pub fn write(&self, buf: &[u8]) -> Result<usize> {
        let mut offset = self.offset.lock();
        if self.status_flags().contains(StatusFlags::O_APPEND) {
            *offset = self.dentry.vnode().len();
        }
        let len = if self.status_flags().contains(StatusFlags::O_DIRECT) {
            self.dentry.vnode().write_direct_at(*offset, buf)?
        } else {
            self.dentry.vnode().write_at(*offset, buf)?
        };

        *offset += len;
        Ok(len)
    }

    pub fn read_to_end(&self, buf: &mut Vec<u8>) -> Result<usize> {
        let len = if self.status_flags().contains(StatusFlags::O_DIRECT) {
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
        let bits = self.status_flags.load(Ordering::Relaxed);
        StatusFlags::from_bits(bits).unwrap()
    }

    pub fn set_status_flags(&self, new_status_flags: StatusFlags) {
        self.status_flags
            .store(new_status_flags.bits(), Ordering::Relaxed);
    }

    pub fn readdir(&self, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        let mut offset = self.offset.lock();
        let read_cnt = self.dentry.vnode().readdir_at(*offset, visitor)?;
        *offset += read_cnt;
        Ok(read_cnt)
    }
}

impl Debug for InodeHandle_ {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("InodeHandle_")
            .field("dentry", &self.dentry)
            .field("offset", &self.offset())
            .field("access_mode", &self.access_mode())
            .field("status_flags", &self.status_flags())
            .finish()
    }
}

/// Methods for both dyn and static
impl<R> InodeHandle<R> {
    pub fn dentry(&self) -> &Arc<Dentry> {
        &self.0.dentry
    }
}
