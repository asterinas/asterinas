// SPDX-License-Identifier: MPL-2.0

use crate::events::IoEvents;
use crate::fs::device::{Device, DeviceId, DeviceType};
use crate::fs::inode_handle::FileIo;
use crate::prelude::*;
use crate::process::signal::Poller;

/// Corresponds to `/dev/tty` in the file system. This device represents the controlling terminal
/// of the session of current process.
pub struct TtyDevice;

impl Device for TtyDevice {
    fn open(&self) -> Result<Option<Arc<dyn FileIo>>> {
        let current = current!();
        let session = current.session().unwrap();

        let Some(terminal) = session.terminal() else {
            return_errno_with_message!(
                Errno::ENOTTY,
                "the session does not have controlling terminal"
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

impl FileIo for TtyDevice {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot read tty device");
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot write tty device");
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        IoEvents::empty()
    }
}
