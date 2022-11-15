use crate::prelude::*;
use core::{any::Any, fmt::Debug};

use super::events::IoEvents;
use super::ioctl::IoctlCmd;

pub type FileDescripter = i32;

/// The basic operations defined on a file
pub trait File: Send + Sync + Debug + Any {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        panic!("read unsupported");
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        panic!("write unsupported");
    }

    fn ioctl(&self, cmd: &mut IoctlCmd) -> Result<i32> {
        panic!("ioctl unsupported");
    }

    fn poll(&self) -> IoEvents {
        IoEvents::empty()
    }
}
