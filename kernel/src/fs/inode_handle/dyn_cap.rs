// SPDX-License-Identifier: MPL-2.0

use aster_rights::TRights;
use inherit_methods_macro::inherit_methods;

use super::*;
use crate::{
    fs::{file_handle::MemoryToMap, utils::Inode},
    prelude::*,
    process::signal::Pollable,
};

impl InodeHandle<Rights> {
    pub fn new(path: Path, access_mode: AccessMode, status_flags: StatusFlags) -> Result<Self> {
        let inode = path.inode();
        inode.check_permission(access_mode.into())?;
        Self::new_unchecked_access(path, access_mode, status_flags)
    }

    pub fn new_unchecked_access(
        path: Path,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Result<Self> {
        let inode = path.inode();
        if inode.type_() == InodeType::Dir && access_mode.is_writable() {
            return_errno_with_message!(Errno::EISDIR, "directory cannot open to write");
        }

        let file_io = if let Some(device) = inode.as_device() {
            device.open()?
        } else {
            None
        };

        let inner = Arc::new(InodeHandle_ {
            path,
            file_io,
            offset: Mutex::new(0),
            access_mode,
            status_flags: AtomicU32::new(status_flags.bits()),
        });
        Ok(Self(inner, Rights::from(access_mode)))
    }

    pub fn to_static<R1: TRights>(self) -> Result<InodeHandle<R1>> {
        let rights = Rights::from_bits(R1::BITS).ok_or(Error::new(Errno::EBADF))?;
        if !self.1.contains(rights) {
            return_errno_with_message!(Errno::EBADF, "check rights failed");
        }
        Ok(InodeHandle(self.0.clone(), R1::new()))
    }

    pub fn readdir(&self, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        if !self.1.contains(Rights::READ) {
            return_errno_with_message!(Errno::EBADF, "file is not readable");
        }
        self.0.readdir(visitor)
    }
}

impl Clone for InodeHandle<Rights> {
    fn clone(&self) -> Self {
        Self(self.0.clone(), self.1)
    }
}

#[inherit_methods(from = "self.0")]
impl Pollable for InodeHandle<Rights> {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents;
}

#[inherit_methods(from = "self.0")]
impl FileLike for InodeHandle<Rights> {
    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32>;
    fn status_flags(&self) -> StatusFlags;
    fn access_mode(&self) -> AccessMode;
    fn metadata(&self) -> Metadata;
    fn mode(&self) -> Result<InodeMode>;
    fn set_mode(&self, mode: InodeMode) -> Result<()>;
    fn owner(&self) -> Result<Uid>;
    fn set_owner(&self, uid: Uid) -> Result<()>;
    fn group(&self) -> Result<Gid>;
    fn set_group(&self, gid: Gid) -> Result<()>;
    fn seek(&self, seek_from: SeekFrom) -> Result<usize>;
    fn mmap(&self) -> Result<MemoryToMap>;

    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        if !self.1.contains(Rights::READ) {
            return_errno_with_message!(Errno::EBADF, "file is not readable");
        }
        self.0.read(writer)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        if !self.1.contains(Rights::WRITE) {
            return_errno_with_message!(Errno::EBADF, "file is not writable");
        }
        self.0.write(reader)
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        if !self.1.contains(Rights::READ) {
            return_errno_with_message!(Errno::EBADF, "file is not readable");
        }
        self.0.read_at(offset, writer)
    }

    fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        if !self.1.contains(Rights::WRITE) {
            return_errno_with_message!(Errno::EBADF, "file is not writable");
        }
        self.0.write_at(offset, reader)
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        if !self.1.contains(Rights::WRITE) {
            return_errno_with_message!(Errno::EINVAL, "file is not writable");
        }
        self.0.resize(new_size)
    }

    fn set_status_flags(&self, new_status_flags: StatusFlags) -> Result<()> {
        self.0.set_status_flags(new_status_flags);
        Ok(())
    }

    fn fallocate(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()> {
        if !self.1.contains(Rights::WRITE) {
            return_errno_with_message!(Errno::EBADF, "file is not writable");
        }
        self.0.fallocate(mode, offset, len)
    }

    fn inode(&self) -> Option<&Arc<dyn Inode>> {
        Some(self.path().inode())
    }
}
