use crate::fs::utils::{IoEvents, IoctlCmd, Metadata, SeekFrom};
use crate::prelude::*;
use crate::tty::get_n_tty;

use core::any::Any;

/// The basic operations defined on a file
pub trait File: Send + Sync + Any {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        panic!("read unsupported");
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        panic!("write unsupported");
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

    fn poll(&self) -> IoEvents {
        IoEvents::empty()
    }

    fn flush(&self) -> Result<()> {
        Ok(())
    }

    fn metadata(&self) -> Metadata {
        panic!("metadata unsupported");
    }

    fn seek(&self, seek_from: SeekFrom) -> Result<usize> {
        panic!("seek unsupported");
    }
}
