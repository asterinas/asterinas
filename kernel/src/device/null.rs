// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use super::*;
use crate::{events::IoEvents, fs::inode_handle::FileIo, prelude::*, process::signal::Poller};

pub struct Null;

impl Device for Null {
    fn type_(&self) -> DeviceType {
        DeviceType::CharDevice
    }

    fn id(&self) -> DeviceId {
        // Same value with Linux
        DeviceId::new(1, 3)
    }

    fn open(&self) -> Result<Option<Arc<dyn FileIo>>> {
        Ok(Some(Arc::new(Null)))
    }
}

impl FileIo for Null {
    fn read(&self, _writer: &mut VmWriter) -> Result<usize> {
        Ok(0)
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        Ok(reader.remain())
    }

    fn poll(&self, mask: IoEvents, poller: Option<&mut Poller>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}
