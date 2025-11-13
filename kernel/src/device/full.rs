// SPDX-License-Identifier: MPL-2.0

use device_id::DeviceId;

use crate::{
    events::IoEvents,
    fs::{
        device::{Device, DeviceType},
        inode_handle::FileIo,
        utils::StatusFlags,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

pub struct Full;

impl Device for Full {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        // The same value as Linux
        DeviceId::new(1, 7)
    }

    fn open(&self) -> Option<Result<Arc<dyn FileIo>>> {
        Some(Ok(Arc::new(Full)))
    }
}

impl Pollable for Full {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl FileIo for Full {
    fn read(&self, writer: &mut VmWriter, _status_flags: StatusFlags) -> Result<usize> {
        let len = writer.avail();
        writer.fill_zeros(len)?;
        Ok(len)
    }

    fn write(&self, _reader: &mut VmReader, _status_flags: StatusFlags) -> Result<usize> {
        return_errno_with_message!(Errno::ENOSPC, "no space left on /dev/full")
    }
}
