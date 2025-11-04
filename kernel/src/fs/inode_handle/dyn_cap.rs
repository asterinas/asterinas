// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::AtomicU32;

use aster_rights::Rights;
use inherit_methods_macro::inherit_methods;

use super::HandleInner;
use crate::{
    events::IoEvents,
    fs::{
        file_handle::{FileLike, Mappable},
        path::Path,
        utils::{
            AccessMode, DirentVisitor, FallocMode, FlockItem, Inode, InodeMode, InodeType,
            IoctlCmd, Metadata, RangeLockItem, RangeLockType, SeekFrom, StatusFlags,
        },
    },
    prelude::*,
    process::{
        signal::{PollHandle, Pollable},
        Gid, Uid,
    },
};

pub struct InodeHandle(HandleInner, Rights);

impl InodeHandle {
    pub fn new(path: Path, access_mode: AccessMode, status_flags: StatusFlags) -> Result<Self> {
        let inode = path.inode();
        if !status_flags.contains(StatusFlags::O_PATH) {
            // "Opening a file or directory with the O_PATH flag requires no permissions on the
            // object itself".
            // Reference: <https://man7.org/linux/man-pages/man2/openat.2.html>
            inode.check_permission(access_mode.into())?;
        }

        Self::new_unchecked_access(path, access_mode, status_flags)
    }

    pub fn new_unchecked_access(
        path: Path,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Result<Self> {
        let inode = path.inode();
        let (file_io, rights) = if status_flags.contains(StatusFlags::O_PATH) {
            (None, Rights::empty())
        } else if inode.type_() == InodeType::Dir && access_mode.is_writable() {
            return_errno_with_message!(Errno::EISDIR, "a directory cannot be opened writable");
        } else {
            let file_io = inode.open(access_mode, status_flags).transpose()?;
            let rights = Rights::from(access_mode);
            (file_io, rights)
        };

        let inner = HandleInner {
            path,
            file_io,
            offset: Mutex::new(0),
            status_flags: AtomicU32::new(status_flags.bits()),
        };
        Ok(Self(inner, rights))
    }

    pub fn readdir(&self, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        if !self.1.contains(Rights::READ) {
            return_errno_with_message!(Errno::EBADF, "the file is not opened readable");
        }
        self.0.readdir(visitor)
    }

    pub fn test_range_lock(&self, lock: RangeLockItem) -> Result<RangeLockItem> {
        if self.1.is_empty() {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        }
        self.0.test_range_lock(lock)
    }

    pub fn set_range_lock(&self, lock: &RangeLockItem, is_nonblocking: bool) -> Result<()> {
        match lock.type_() {
            RangeLockType::ReadLock => {
                if !self.1.contains(Rights::READ) {
                    return_errno_with_message!(Errno::EBADF, "the file is not opened readable");
                }
            }
            RangeLockType::WriteLock => {
                if !self.1.contains(Rights::WRITE) {
                    return_errno_with_message!(Errno::EBADF, "the file is not opened writable");
                }
            }
            RangeLockType::Unlock => {
                if self.1.is_empty() {
                    return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
                }
            }
        }
        self.0.set_range_lock(lock, is_nonblocking)
    }

    pub fn set_flock(&self, lock: FlockItem, is_nonblocking: bool) -> Result<()> {
        if self.1.is_empty() {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        }
        self.0.set_flock(lock, is_nonblocking)
    }

    pub fn unlock_flock(&self) -> Result<()> {
        if self.1.is_empty() {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        }
        self.0.unlock_flock(self);
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.0.path
    }

    pub fn offset(&self) -> usize {
        self.0.offset()
    }
}

#[inherit_methods(from = "self.0")]
impl Pollable for InodeHandle {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents;
}

#[inherit_methods(from = "self.0")]
impl FileLike for InodeHandle {
    fn status_flags(&self) -> StatusFlags;
    fn metadata(&self) -> Metadata;
    fn mode(&self) -> Result<InodeMode>;
    fn set_mode(&self, mode: InodeMode) -> Result<()>;
    fn owner(&self) -> Result<Uid>;
    fn set_owner(&self, uid: Uid) -> Result<()>;
    fn group(&self) -> Result<Gid>;
    fn set_group(&self, gid: Gid) -> Result<()>;

    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        if !self.1.contains(Rights::READ) {
            return_errno_with_message!(Errno::EBADF, "the file is not opened readable");
        }
        self.0.read(writer)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        if !self.1.contains(Rights::WRITE) {
            return_errno_with_message!(Errno::EBADF, "the file is not opened writable");
        }
        self.0.write(reader)
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        if !self.1.contains(Rights::READ) {
            return_errno_with_message!(Errno::EBADF, "the file is not opened readable");
        }
        self.0.read_at(offset, writer)
    }

    fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        if !self.1.contains(Rights::WRITE) {
            return_errno_with_message!(Errno::EBADF, "the file is not opened writable");
        }
        self.0.write_at(offset, reader)
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        if self.1.is_empty() {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        }
        self.0.ioctl(cmd, arg)
    }

    fn mappable(&self) -> Result<Mappable> {
        if self.1.is_empty() {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        }
        self.0.mappable()
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        if self.1.is_empty() {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        }
        if !self.1.contains(Rights::WRITE) {
            return_errno_with_message!(Errno::EINVAL, "the file is not opened writable");
        }
        self.0.resize(new_size)
    }

    fn set_status_flags(&self, new_status_flags: StatusFlags) -> Result<()> {
        self.0.set_status_flags(new_status_flags);
        Ok(())
    }

    fn access_mode(&self) -> AccessMode {
        self.1.into()
    }

    fn seek(&self, seek_from: SeekFrom) -> Result<usize> {
        if self.1.is_empty() {
            return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
        }
        self.0.seek(seek_from)
    }

    fn fallocate(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()> {
        if !self.1.contains(Rights::WRITE) {
            return_errno_with_message!(Errno::EBADF, "the file is not opened writable");
        }
        self.0.fallocate(mode, offset, len)
    }

    fn inode(&self) -> &Arc<dyn Inode> {
        self.0.inode()
    }
}

impl Drop for InodeHandle {
    fn drop(&mut self) {
        self.0.release_range_locks();
        self.0.unlock_flock(self);
    }
}
