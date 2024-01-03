// SPDX-License-Identifier: MPL-2.0

//! Opend Inode-backed File Handle

mod dyn_cap;
mod static_cap;

use core::sync::atomic::{AtomicU32, Ordering};

use crate::events::IoEvents;
use crate::fs::device::Device;
use crate::fs::file_handle::FileLike;
use crate::fs::utils::{
    AccessMode, Dentry, DirentVisitor, InodeType, IoctlCmd, Metadata, SeekFrom, StatusFlags,
};
use crate::prelude::*;
use crate::process::signal::Poller;
use aster_rights::Rights;

#[derive(Debug)]
pub struct InodeHandle<R = Rights>(Arc<InodeHandle_>, R);

struct InodeHandle_ {
    dentry: Arc<Dentry>,
    /// `file_io` is Similar to `file_private` field in `file` structure in linux. If
    /// `file_io` is Some, typical file operations including `read`, `write`, `poll`,
    /// `ioctl` will be provided by `file_io`, instead of `dentry`.
    file_io: Option<Arc<dyn FileIo>>,
    offset: Mutex<usize>,
    access_mode: AccessMode,
    status_flags: AtomicU32,
}

impl InodeHandle_ {
    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        let mut offset = self.offset.lock();

        if let Some(ref file_io) = self.file_io {
            return file_io.read(buf);
        }

        let len = if self.status_flags().contains(StatusFlags::O_DIRECT) {
            self.dentry.inode().read_direct_at(*offset, buf)?
        } else {
            self.dentry.inode().read_at(*offset, buf)?
        };

        *offset += len;
        Ok(len)
    }

    pub fn write(&self, buf: &[u8]) -> Result<usize> {
        let mut offset = self.offset.lock();

        if let Some(ref file_io) = self.file_io {
            return file_io.write(buf);
        }

        if self.status_flags().contains(StatusFlags::O_APPEND) {
            *offset = self.dentry.inode_len();
        }
        let len = if self.status_flags().contains(StatusFlags::O_DIRECT) {
            self.dentry.inode().write_direct_at(*offset, buf)?
        } else {
            self.dentry.inode().write_at(*offset, buf)?
        };

        *offset += len;
        Ok(len)
    }

    pub fn read_to_end(&self, buf: &mut Vec<u8>) -> Result<usize> {
        if self.file_io.is_some() {
            return_errno_with_message!(Errno::EINVAL, "file io does not support read to end");
        }

        let len = if self.status_flags().contains(StatusFlags::O_DIRECT) {
            self.dentry.inode().read_direct_to_end(buf)?
        } else {
            self.dentry.inode().read_to_end(buf)?
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
                let file_size = self.dentry.inode_len() as isize;
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
        self.dentry.inode_len()
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
        let read_cnt = self.dentry.inode().readdir_at(*offset, visitor)?;
        *offset += read_cnt;
        Ok(read_cnt)
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        if let Some(ref file_io) = self.file_io {
            return file_io.poll(mask, poller);
        }

        self.dentry.inode().poll(mask, poller)
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        if let Some(ref file_io) = self.file_io {
            return file_io.ioctl(cmd, arg);
        }

        self.dentry.inode().ioctl(cmd, arg)
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

pub trait FileIo: Send + Sync + 'static {
    fn read(&self, buf: &mut [u8]) -> Result<usize>;

    fn write(&self, buf: &[u8]) -> Result<usize>;

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents;

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        return_errno_with_message!(Errno::EINVAL, "ioctl is not supported");
    }
}
