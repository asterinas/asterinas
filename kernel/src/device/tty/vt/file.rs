// SPDX-License-Identifier: MPL-2.0

use inherit_methods_macro::inherit_methods;

use crate::{
    device::tty::{Tty, vt::VtDriver},
    events::IoEvents,
    fs::{
        inode_handle::FileIo,
        utils::{InodeIo, StatusFlags},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::ioctl::RawIoctl,
};

/// The file representation of a virtual terminal (VT) device.
pub(in crate::device::tty::vt) struct VtFile(Arc<Tty<VtDriver>>);

impl VtFile {
    pub(in crate::device::tty::vt) fn new(tty: Arc<Tty<VtDriver>>) -> Result<VtFile> {
        tty.driver().inc_open();
        Ok(VtFile(tty))
    }
}

impl Drop for VtFile {
    fn drop(&mut self) {
        self.0.driver().dec_open();
    }
}

#[inherit_methods(from = "self.0")]
impl Pollable for VtFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents;
}

impl InodeIo for VtFile {
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
impl FileIo for VtFile {
    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32>;

    fn check_seekable(&self) -> Result<()> {
        return_errno_with_message!(Errno::ESPIPE, "the inode is a TTY");
    }

    fn is_offset_aware(&self) -> bool {
        false
    }
}
