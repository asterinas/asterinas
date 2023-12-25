//! Opend File Handle

use crate::events::{IoEvents, Observer};
use crate::fs::device::Device;
use crate::fs::utils::{AccessMode, IoctlCmd, Metadata, SeekFrom, StatusFlags};
use crate::net::socket::Socket;
use crate::prelude::*;
use crate::process::signal::Poller;

use core::any::Any;

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

    fn flush(&self) -> Result<()> {
        Ok(())
    }

    fn metadata(&self) -> Metadata {
        panic!("metadata unsupported");
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

    fn as_socket(&self) -> Option<&dyn Socket> {
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
