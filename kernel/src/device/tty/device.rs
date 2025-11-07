// SPDX-License-Identifier: MPL-2.0

use crate::{
    events::IoEvents,
    fs::{
        device::{Device, DeviceId, DeviceType},
        inode_handle::FileIo,
        utils::StatusFlags,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

/// Corresponds to `/dev/tty` in the file system. This device represents the controlling terminal
/// of the session of current process.
pub struct TtyDevice;

impl Device for TtyDevice {
    fn open(&self) -> Option<Result<Arc<dyn FileIo>>> {
        let Some(terminal) = current!().terminal() else {
            return Some(Err(Error::with_message(
                Errno::ENOTTY,
                "the process does not have a controlling terminal",
            )));
        };

        terminal.open()
    }

    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        DeviceId::new(5, 0)
    }
}

impl Pollable for TtyDevice {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        IoEvents::empty()
    }
}

impl FileIo for TtyDevice {
    fn read(&self, _writer: &mut VmWriter, _status_flags: StatusFlags) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot read tty device");
    }

    fn write(&self, _reader: &mut VmReader, _status_flags: StatusFlags) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot write tty device");
    }
}
