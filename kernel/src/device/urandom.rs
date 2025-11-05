// SPDX-License-Identifier: MPL-2.0

use device_id::DeviceId;

use crate::{
    events::IoEvents,
    fs::{
        device::{Device, DeviceType},
        inode_handle::FileIo,
        utils::{InodeIo, StatusFlags},
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::random::getrandom,
};

pub struct Urandom;

impl Urandom {
    pub fn getrandom(writer: &mut VmWriter) -> Result<usize> {
        const IO_CAPABILITY: usize = 4096;

        if !writer.has_avail() {
            return Ok(0);
        }

        let mut buffer = vec![0; writer.avail().min(IO_CAPABILITY)];
        let mut written_bytes = 0;

        while writer.has_avail() {
            getrandom(&mut buffer[..writer.avail().min(IO_CAPABILITY)]);
            match writer.write_fallible(&mut VmReader::from(buffer.as_slice())) {
                Ok(len) => written_bytes += len,
                Err((err, 0)) if written_bytes == 0 => return Err(err.into()),
                Err((_, len)) => return Ok(written_bytes + len),
            }
        }

        Ok(written_bytes)
    }
}

impl Device for Urandom {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        // The same value as Linux
        DeviceId::new(1, 9)
    }

    fn open(&self) -> Result<Box<dyn FileIo>> {
        Ok(Box::new(Self))
    }
}

impl Pollable for Urandom {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl InodeIo for Urandom {
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

impl FileIo for Urandom {
    fn check_seekable(&self) -> Result<()> {
        Ok(())
    }

    fn is_offset_aware(&self) -> bool {
        false
    }
}
