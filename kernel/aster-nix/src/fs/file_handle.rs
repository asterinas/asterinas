// SPDX-License-Identifier: MPL-2.0

//! Opend File Handle

use crate::{
    events::{IoEvents, Observer},
    fs::{
        device::Device,
        utils::{AccessMode, InodeMode, IoctlCmd, Metadata, SeekFrom, StatusFlags},
    },
    net::socket::Socket,
    prelude::*,
    process::{signal::Poller, Gid, Uid},
};

/// The basic operations defined on a file
pub trait FileLike: Send + Sync + Any {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "read is not supported");
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "write is not supported");
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        return_errno_with_message!(Errno::EINVAL, "ioctl is not supported");
    }

    fn poll(&self, _mask: IoEvents, _poller: Option<&Poller>) -> IoEvents {
        IoEvents::empty()
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
        return_errno_with_message!(Errno::EINVAL, "seek is not supported");
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

    fn unregister_observer(
        &self,
        observer: &Weak<dyn Observer<IoEvents>>,
    ) -> Result<Weak<dyn Observer<IoEvents>>> {
        return_errno_with_message!(Errno::EINVAL, "unregister_observer is not supported")
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
