// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

//! Opend File Handle

use crate::{
    events::{IoEvents, Observer},
    fs::{
        device::Device,
        utils::{AccessMode, InodeMode, IoctlCmd, Metadata, SeekFrom, StatusFlags},
    },
    net::socket::Socket,
    prelude::*,
    process::{signal::Pollable, Gid, Uid},
};

/// The basic operations defined on a file
pub trait FileLike: Pollable + Send + Sync + Any {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        return_errno_with_message!(Errno::EBADF, "the file is not valid for reading");
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        return_errno_with_message!(Errno::EBADF, "the file is not valid for writing");
    }

    /// Read at the given file offset.
    ///
    /// The file must be seekable to support `read_at`.
    /// Unlike [`read`], `read_at` will not change the file offset.
    ///
    /// [`read`]: FileLike::read
    fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        return_errno_with_message!(Errno::ESPIPE, "read_at is not supported");
    }

    /// Write at the given file offset.
    ///
    /// The file must be seekable to support `write_at`.
    /// Unlike [`write`], `write_at` will not change the file offset.
    /// If the file is append-only, the `offset` will be ignored.
    ///
    /// [`write`]: FileLike::write
    fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        return_errno_with_message!(Errno::ESPIPE, "write_at is not supported");
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        return_errno_with_message!(Errno::EINVAL, "ioctl is not supported");
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        return_errno_with_message!(Errno::EINVAL, "resize is not supported");
    }

    fn flush(&self) -> Result<()> {
        Ok(())
    }

    fn metadata(&self) -> Metadata {
        panic!("metadata unsupported");
    }

    fn mode(&self) -> Result<InodeMode> {
        return_errno_with_message!(Errno::EINVAL, "mode is not supported");
    }

    fn set_mode(&self, mode: InodeMode) -> Result<()> {
        return_errno_with_message!(Errno::EINVAL, "set_mode is not supported");
    }

    fn owner(&self) -> Result<Uid> {
        return_errno_with_message!(Errno::EPERM, "owner is not supported");
    }

    fn set_owner(&self, uid: Uid) -> Result<()> {
        return_errno_with_message!(Errno::EPERM, "set_owner is not supported");
    }

    fn group(&self) -> Result<Gid> {
        return_errno_with_message!(Errno::EPERM, "group is not supported");
    }

    fn set_group(&self, gid: Gid) -> Result<()> {
        return_errno_with_message!(Errno::EPERM, "set_group is not supported");
    }

    fn status_flags(&self) -> StatusFlags {
        StatusFlags::empty()
    }

    fn set_status_flags(&self, _new_flags: StatusFlags) -> Result<()> {
        return_errno_with_message!(Errno::EINVAL, "set_status_flags is not supported");
    }

    fn access_mode(&self) -> AccessMode {
        AccessMode::O_RDWR
    }

    fn seek(&self, seek_from: SeekFrom) -> Result<usize> {
        return_errno_with_message!(Errno::ESPIPE, "seek is not supported");
    }

    fn clean_for_close(&self) -> Result<()> {
        self.flush()?;
        Ok(())
    }

    fn register_observer(
        &self,
        observer: Weak<dyn Observer<IoEvents>>,
        mask: IoEvents,
    ) -> Result<()> {
        return_errno_with_message!(Errno::EINVAL, "register_observer is not supported")
    }

    #[must_use]
    fn unregister_observer(
        &self,
        observer: &Weak<dyn Observer<IoEvents>>,
    ) -> Option<Weak<dyn Observer<IoEvents>>> {
        None
    }

    fn as_socket(self: Arc<Self>) -> Option<Arc<dyn Socket>> {
        None
    }

    fn as_device(&self) -> Option<Arc<dyn Device>> {
        None
    }
}

impl dyn FileLike {
    pub fn downcast_ref<T: FileLike>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref::<T>()
    }
}
