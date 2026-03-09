// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use inherit_methods_macro::inherit_methods;
use ostd::mm::{VmReader, VmWriter};

use super::{Tty, TtyDriver};
use crate::{
    events::IoEvents,
    fs::{
        file::{FileIo, StatusFlags},
        vfs::inode::InodeIo,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::ioctl::RawIoctl,
};

pub(super) struct TtyFile<D>(Arc<Tty<D>>);

impl<D: TtyDriver> TtyFile<D> {
    pub(super) fn new(tty: Arc<Tty<D>>) -> Self {
        Self(tty)
    }
}

#[inherit_methods(from = "self.0")]
impl<D: TtyDriver> Pollable for TtyFile<D> {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents;
}

impl<D: TtyDriver> InodeIo for TtyFile<D> {
    fn read_at(
        &self,
        _offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        self.0.read(writer, status_flags)
    }

    fn write_at(
        &self,
        _offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        self.0.write(reader, status_flags)
    }
}

#[inherit_methods(from = "self.0")]
impl<D: TtyDriver> FileIo for TtyFile<D> {
    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32>;

    fn check_seekable(&self) -> Result<()> {
        return_errno_with_message!(Errno::ESPIPE, "the inode is a TTY");
    }

    fn is_offset_aware(&self) -> bool {
        false
    }
}
