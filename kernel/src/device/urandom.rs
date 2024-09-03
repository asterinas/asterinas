// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use crate::{
    events::IoEvents,
    fs::{
        device::{Device, DeviceId, DeviceType},
        inode_handle::FileIo,
    },
    prelude::*,
    process::signal::Poller,
    util::random::getrandom,
};

pub struct Urandom;

impl Urandom {
    pub fn getrandom(buf: &mut [u8]) -> Result<usize> {
        getrandom(buf)?;
        Ok(buf.len())
    }
}

impl Device for Urandom {
    fn type_(&self) -> DeviceType {
        DeviceType::CharDevice
    }

    fn id(&self) -> DeviceId {
        // The same value as Linux
        DeviceId::new(1, 9)
    }

    fn open(&self) -> Result<Option<Arc<dyn FileIo>>> {
        Ok(Some(Arc::new(Urandom)))
    }
}

impl FileIo for Urandom {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        let mut buf = vec![0; writer.avail()];
        Self::getrandom(buf.as_mut_slice())
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        Ok(reader.remain())
    }

    fn poll(&self, mask: IoEvents, poller: Option<&mut Poller>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}
