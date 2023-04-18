//! Opend File Handle

use crate::events::Observer;
use crate::fs::utils::{IoEvents, IoctlCmd, Metadata, Poller, SeekFrom};
use crate::prelude::*;
use crate::tty::get_n_tty;

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
        match cmd {
            IoctlCmd::TCGETS => {
                // FIXME: only a work around
                let tty = get_n_tty();
                tty.ioctl(cmd, arg)
            }
            _ => panic!("Ioctl unsupported"),
        }
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

    fn seek(&self, seek_from: SeekFrom) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "seek is not supported");
    }

    fn clean_for_close(&self) -> Result<()> {
        self.flush()?;
        Ok(())
    }

    fn register_observer(
        &self,
        observer: Arc<dyn Observer<IoEvents>>,
        mask: IoEvents,
    ) -> Result<()> {
        return_errno_with_message!(Errno::EINVAL, "register_observer is not supported")
    }

    fn unregister_observer(
        &self,
        observer: &Arc<dyn Observer<IoEvents>>,
    ) -> Result<Arc<dyn Observer<IoEvents>>> {
        return_errno_with_message!(Errno::EINVAL, "unregister_observer is not supported")
    }

    fn as_any_ref(&self) -> &dyn Any;
}

impl dyn FileLike {
    pub fn downcast_ref<T: FileLike>(&self) -> Option<&T> {
        self.as_any_ref().downcast_ref::<T>()
    }
}
