// SPDX-License-Identifier: MPL-2.0

use inherit_methods_macro::inherit_methods;

use crate::{
    device::PtySlave,
    events::IoEvents,
    fs::{
        file::{FileIo, StatusFlags},
        vfs::inode::InodeIo,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::ioctl::RawIoctl,
};

/// The file for a pseudoterminal slave.
pub(super) struct PtySlaveFile(Arc<PtySlave>);

impl PtySlaveFile {
    pub(super) fn new(slave: Arc<PtySlave>) -> Result<PtySlaveFile> {
        // Reference: <https://elixir.bootlin.com/linux/v6.17/source/drivers/tty/pty.c#L220>.

        // Hold the lock at first to avoid race condition.
        let mut opened_slaves = slave.driver().opened_slaves().lock();

        let master_flags = slave.driver().tty_flags();
        if master_flags.is_pty_locked() || slave.tty_flags().is_other_closed() {
            return_errno_with_message!(
                Errno::EIO,
                "a pty slave cannot be opened when the pty master is locked or closed"
            );
        }
        master_flags.clear_other_closed();

        *opened_slaves += 1;
        drop(opened_slaves);

        slave.driver().pollee().invalidate();
        Ok(PtySlaveFile(slave))
    }
}

impl Drop for PtySlaveFile {
    fn drop(&mut self) {
        let driver = self.0.driver();

        let mut opened_slaves = driver.opened_slaves().lock();
        *opened_slaves -= 1;

        if *opened_slaves == 0 {
            driver.tty_flags().set_other_closed();
            driver.pollee().notify(IoEvents::HUP);
        }
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
    fn ioctl(&self, raw_ioctl: RawIoctl) -> Result<i32>;

    fn check_seekable(&self) -> Result<()> {
        return_errno_with_message!(Errno::ESPIPE, "the inode is a TTY");
    }

    fn is_offset_aware(&self) -> bool {
        false
    }
}
