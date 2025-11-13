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

pub struct Null;

impl Device for Null {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        // The same value as Linux
        DeviceId::new(1, 3)
    }

    fn open(&self) -> Option<Result<Arc<dyn FileIo>>> {
        Some(Ok(Arc::new(Null)))
    }
}

impl Pollable for Null {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl FileIo for Null {
    fn read(&self, _writer: &mut VmWriter, _status_flags: StatusFlags) -> Result<usize> {
        Ok(0)
    }

    fn write(&self, reader: &mut VmReader, _status_flags: StatusFlags) -> Result<usize> {
        let len = reader.remain();
        reader.skip(len);
        Ok(len)
    }
}
