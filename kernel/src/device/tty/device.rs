// SPDX-License-Identifier: MPL-2.0

use device_id::{DeviceId, MajorId, MinorId};

use crate::{
    fs::{
        device::{Device, DeviceType},
        inode_handle::FileIo,
    },
    prelude::*,
};

/// Corresponds to `/dev/tty` in the file system. This device represents the controlling terminal
/// of the session of current process.
pub struct TtyDevice;

impl Device for TtyDevice {
    fn open(&self) -> Result<Box<dyn FileIo>> {
        let Some(terminal) = current!().terminal() else {
            return_errno_with_message!(
                Errno::ENOTTY,
                "the process does not have a controlling terminal"
            );
        };

        terminal.open()
    }

    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        DeviceId::new(MajorId::new(5), MinorId::new(0))
    }
}
