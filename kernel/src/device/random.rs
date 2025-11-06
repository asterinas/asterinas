// SPDX-License-Identifier: MPL-2.0

use device_id::DeviceId;

use super::Urandom;
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

pub struct Random;

impl Random {
    pub fn getrandom(writer: &mut VmWriter) -> Result<usize> {
        // TODO: Support true randomness by collecting environment noise.
        Urandom::getrandom(writer)
    }
}

impl Device for Random {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        // The same value as Linux
        DeviceId::new(1, 8)
    }

    fn open(&self) -> Result<Box<dyn FileIo>> {
        Ok(Box::new(Self))
    }
}

impl Pollable for Random {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl FileIo for Random {
    fn read(&self, writer: &mut VmWriter, _status_flags: StatusFlags) -> Result<usize> {
        Self::getrandom(writer)
    }

    fn write(&self, reader: &mut VmReader, _status_flags: StatusFlags) -> Result<usize> {
        let len = reader.remain();
        reader.skip(len);
        Ok(len)
    }
}
