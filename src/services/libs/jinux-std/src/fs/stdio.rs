use crate::prelude::*;
use crate::tty::{get_n_tty, Tty};

use super::file_handle::File;
use super::file_table::FileDescripter;
use super::utils::{InodeMode, InodeType, IoEvents, Metadata, SeekFrom};

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

    fn seek(&self, seek_from: SeekFrom) -> Result<usize> {
        // TODO: do real seek
        Ok(0)
    }

    fn metadata(&self) -> Metadata {
        Metadata {
            dev: Default::default(),
            ino: 0,
            size: 0,
            blk_size: 1024,
            blocks: 0,
            atime: Default::default(),
            mtime: Default::default(),
            ctime: Default::default(),
            type_: InodeType::CharDevice,
            mode: InodeMode::from_bits_truncate(0o620),
            nlinks: 1,
            uid: 0,
            gid: 0,
            rdev: 0,
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

    fn seek(&self, seek_from: SeekFrom) -> Result<usize> {
        // TODO: do real seek
        Ok(0)
    }

    fn metadata(&self) -> Metadata {
        Metadata {
            dev: Default::default(),
            ino: 0,
            size: 0,
            blk_size: 1024,
            blocks: 0,
            atime: Default::default(),
            mtime: Default::default(),
            ctime: Default::default(),
            type_: InodeType::CharDevice,
            mode: InodeMode::from_bits_truncate(0o620),
            nlinks: 1,
            uid: 0,
            gid: 0,
            rdev: 0,
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

    fn seek(&self, seek_from: SeekFrom) -> Result<usize> {
        // TODO: do real seek
        Ok(0)
    }

    fn metadata(&self) -> Metadata {
        Metadata {
            dev: Default::default(),
            ino: 0,
            size: 0,
            blk_size: 1024,
            blocks: 0,
            atime: Default::default(),
            mtime: Default::default(),
            ctime: Default::default(),
            type_: InodeType::CharDevice,
            mode: InodeMode::from_bits_truncate(0o620),
            nlinks: 1,
            uid: 0,
            gid: 0,
            rdev: 0,
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
