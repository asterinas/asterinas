// SPDX-License-Identifier: MPL-2.0

use device_id::DeviceId;

use super::Urandom;
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

impl InodeIo for Random {
    fn read_at(
        &self,
        _offset: usize,
        writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        Self::getrandom(writer)
    }

    fn write_at(
        &self,
        _offset: usize,
        reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        let len = reader.remain();
        reader.skip(len);
        Ok(len)
    }
}

impl FileIo for Random {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        false
    }
}
