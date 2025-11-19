// SPDX-License-Identifier: MPL-2.0

use device_id::{DeviceId, MajorId, MinorId};

use crate::{
    events::IoEvents,
    fs::{
        device::{Device, DeviceType},
        inode_handle::FileIo,
        utils::{InodeIo, StatusFlags},
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
        DeviceId::new(MajorId::new(1), MinorId::new(7))
    }

    fn open(&self) -> Result<Box<dyn FileIo>> {
        Ok(Box::new(Self))
    }
}

impl Pollable for Full {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl InodeIo for Full {
    fn read_at(
        &self,
        _offset: usize,
        writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        let len = writer.avail();
        writer.fill_zeros(len)?;
        Ok(len)
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::ENOSPC, "no space left on /dev/full")
    }
}

impl FileIo for Full {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        false
    }
}
