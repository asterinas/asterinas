use crate::prelude::*;
use crate::tty::{get_n_tty, Tty};

use super::file_handle::File;
use super::file_table::FileDescripter;
use super::utils::IoEvents;

pub const FD_STDIN: FileDescripter = 0;
pub const FD_STDOUT: FileDescripter = 1;
pub const FD_STDERR: FileDescripter = 2;

pub struct Stdin {
    console: Option<Arc<Tty>>,
}

pub struct Stdout {
    console: Option<Arc<Tty>>,
}

pub struct Stderr {
    console: Option<Arc<Tty>>,
}

impl File for Stdin {
    fn poll(&self) -> IoEvents {
        if let Some(console) = self.console.as_ref() {
            console.poll()
        } else {
            todo!()
        }
    }

    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        if let Some(console) = self.console.as_ref() {
            console.read(buf)
        } else {
            todo!()
        }
    }

    fn ioctl(&self, cmd: super::utils::IoctlCmd, arg: usize) -> Result<i32> {
        if let Some(console) = self.console.as_ref() {
            console.ioctl(cmd, arg)
        } else {
            todo!()
        }
    }
}
impl File for Stdout {
    fn ioctl(&self, cmd: super::utils::IoctlCmd, arg: usize) -> Result<i32> {
        if let Some(console) = self.console.as_ref() {
            console.ioctl(cmd, arg)
        } else {
            todo!()
        }
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        if let Some(console) = self.console.as_ref() {
            console.write(buf)
        } else {
            todo!()
        }
    }
}

impl File for Stderr {
    fn ioctl(&self, cmd: super::utils::IoctlCmd, arg: usize) -> Result<i32> {
        if let Some(console) = self.console.as_ref() {
            console.ioctl(cmd, arg)
        } else {
            todo!()
        }
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        if let Some(console) = self.console.as_ref() {
            console.write(buf)
        } else {
            todo!()
        }
    }
}

impl Stdin {
    /// FIXME: console should be file under devfs.
    /// reimplement the function when devfs is enabled.
    pub fn new_with_default_console() -> Self {
        let console = get_n_tty();
        Self {
            console: Some(console.clone()),
        }
    }
}

impl Stdout {
    /// FIXME: console should be file under devfs.
    /// reimplement the function when devfs is enabled.
    pub fn new_with_default_console() -> Self {
        let console = get_n_tty();
        Self {
            console: Some(console.clone()),
        }
    }
}

impl Stderr {
    /// FIXME: console should be file under devfs.
    /// reimplement the function when devfs is enabled.
    pub fn new_with_default_console() -> Self {
        let console = get_n_tty();
        Self {
            console: Some(console.clone()),
        }
    }
}
