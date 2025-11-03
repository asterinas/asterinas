// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use inherit_methods_macro::inherit_methods;

use crate::{
    device::PtySlave,
    events::IoEvents,
    fs::{
        inode_handle::FileIo,
        utils::{IoctlCmd, StatusFlags},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

/// The file for a pseudoterminal slave.
pub struct PtySlaveFile(Arc<PtySlave>);

impl PtySlaveFile {
    pub fn new(slave: Arc<PtySlave>) -> PtySlaveFile {
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
impl FileIo for PtySlaveFile {
    fn read(&self, writer: &mut VmWriter, status_flags: StatusFlags) -> Result<usize>;
    fn write(&self, reader: &mut VmReader, status_flags: StatusFlags) -> Result<usize>;
    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32>;
}

#[inherit_methods(from = "self.0")]
impl Pollable for PtySlaveFile {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents;
}
