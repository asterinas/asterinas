use super::events::IoEvents;
use crate::prelude::*;
use crate::tty::{get_console, Tty};

use super::file::{File, FileDescripter};

pub const FD_STDIN: FileDescripter = 0;
pub const FD_STDOUT: FileDescripter = 1;
pub const FD_STDERR: FileDescripter = 2;

pub struct Stdin {
    console: Option<Arc<Tty>>,
    bind_to_console: bool,
}

pub struct Stdout {
    console: Option<Arc<Tty>>,
    bind_to_console: bool,
}

pub struct Stderr {
    console: Option<Arc<Tty>>,
    bind_to_console: bool,
}

impl File for Stdin {
    fn poll(&self) -> IoEvents {
        if self.bind_to_console {
            let console = self.console.as_ref().unwrap();
            console.poll()
        } else {
            todo!()
        }
    }

    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        if self.bind_to_console {
            let console = self.console.as_ref().unwrap();
            console.read(buf)
        } else {
            todo!()
        }
    }

    fn ioctl(&self, cmd: super::ioctl::IoctlCmd, arg: usize) -> Result<i32> {
        if self.bind_to_console {
            let console = self.console.as_ref().unwrap();
            console.ioctl(cmd, arg)
        } else {
            todo!()
        }
    }
}
impl File for Stdout {
    fn ioctl(&self, cmd: super::ioctl::IoctlCmd, arg: usize) -> Result<i32> {
        if self.bind_to_console {
            let console = self.console.as_ref().unwrap();
            console.ioctl(cmd, arg)
        } else {
            todo!()
        }
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        if self.bind_to_console {
            let console = self.console.as_ref().unwrap();
            console.write(buf)
        } else {
            todo!()
        }
    }
}

impl File for Stderr {
    fn ioctl(&self, cmd: super::ioctl::IoctlCmd, arg: usize) -> Result<i32> {
        if self.bind_to_console {
            let console = self.console.as_ref().unwrap();
            console.ioctl(cmd, arg)
        } else {
            todo!()
        }
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        if self.bind_to_console {
            let console = self.console.as_ref().unwrap();
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
        let console = get_console();
        Self {
            console: Some(console.clone()),
            bind_to_console: true,
        }
    }
}

impl Stdout {
    /// FIXME: console should be file under devfs.
    /// reimplement the function when devfs is enabled.
    pub fn new_with_default_console() -> Self {
        let console = get_console();
        Self {
            console: Some(console.clone()),
            bind_to_console: true,
        }
    }
}

impl Stderr {
    /// FIXME: console should be file under devfs.
    /// reimplement the function when devfs is enabled.
    pub fn new_with_default_console() -> Self {
        let console = get_console();
        Self {
            console: Some(console.clone()),
            bind_to_console: true,
        }
    }
}
