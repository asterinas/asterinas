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

pub struct Zero;

impl Device for Zero {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        // The same value as Linux
        DeviceId::new(1, 5)
    }

    fn open(&self) -> Result<Box<dyn FileIo>> {
        Ok(Box::new(Self))
    }
}

impl Pollable for Zero {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl FileIo for Zero {
    fn read(&self, writer: &mut VmWriter, _status_flags: StatusFlags) -> Result<usize> {
        let read_len = writer.fill_zeros(writer.avail())?;
        Ok(read_len)
    }

    fn write(&self, reader: &mut VmReader, _status_flags: StatusFlags) -> Result<usize> {
        Ok(reader.remain())
    }
}
