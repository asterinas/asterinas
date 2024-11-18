// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use crate::{
    events::IoEvents,
    fs::{
        device::{Device, DeviceId, DeviceType},
        inode_handle::FileIo,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable},
    util::random::getrandom,
};

pub struct Random;

impl Random {
    pub fn getrandom(buf: &mut [u8]) -> Result<usize> {
        getrandom(buf)?;
        Ok(buf.len())
    }
}

impl Device for Random {
    fn type_(&self) -> DeviceType {
        DeviceType::CharDevice
    }

    fn id(&self) -> DeviceId {
        // The same value as Linux
        DeviceId::new(1, 8)
    }

    fn open(&self) -> Result<Option<Arc<dyn FileIo>>> {
        Ok(Some(Arc::new(Random)))
    }
}

impl Pollable for Random {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl FileIo for Random {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        let mut buf = vec![0; writer.avail()];
        let size = Self::getrandom(buf.as_mut_slice());
        writer.write_fallible(&mut buf.as_slice().into())?;
        size
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        Ok(reader.remain())
    }
}
