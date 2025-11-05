// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use inherit_methods_macro::inherit_methods;

use crate::{
    device::PtySlave,
    events::IoEvents,
    fs::{
        inode_handle::FileIo,
        utils::{InodeIo, IoctlCmd, StatusFlags},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

/// The file for a pseudoterminal slave.
pub(super) struct PtySlaveFile(Arc<PtySlave>);

impl PtySlaveFile {
    pub(super) fn new(slave: Arc<PtySlave>) -> PtySlaveFile {
        slave
            .driver()
            .opened_slaves()
            .fetch_add(1, Ordering::Relaxed);
        slave.driver().pollee().invalidate();
        PtySlaveFile(slave)
    }
}

impl Drop for PtySlaveFile {
    fn drop(&mut self) {
        let driver = self.0.driver();
        driver.opened_slaves().fetch_sub(1, Ordering::Relaxed);
        driver.pollee().notify(IoEvents::HUP);
    }
}

#[inherit_methods(from = "self.0")]
impl Pollable for PtySlaveFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents;
}

impl InodeIo for PtySlaveFile {
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
impl FileIo for PtySlaveFile {
    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32>;

    fn is_seekable(&self) -> Result<bool> {
        return_errno_with_message!(Errno::ESPIPE, "the inode is a TTY");
    }
}
