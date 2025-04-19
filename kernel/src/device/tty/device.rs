// SPDX-License-Identifier: MPL-2.0

use crate::{
    events::IoEvents,
    fs::{
        device::{Device, DeviceId, DeviceType},
        inode_handle::FileIo,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

/// Corresponds to `/dev/tty` in the file system. This device represents the controlling terminal
/// of the session of current process.
pub struct TtyDevice;

impl Device for TtyDevice {
    fn open(&self) -> Result<Option<Arc<dyn FileIo>>> {
        let Some(terminal) = current!().terminal() else {
            return_errno_with_message!(
                Errno::ENOTTY,
                "the process does not have a controlling terminal"
            );
        };

        Ok(Some(terminal as Arc<dyn FileIo>))
    }

    fn type_(&self) -> DeviceType {
        DeviceType::CharDevice
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
    fn read(&self, _writer: &mut VmWriter) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot read tty device");
    }

    fn write(&self, _reader: &mut VmReader) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot write tty device");
    }
}
