// SPDX-License-Identifier: MPL-2.0

use crate::events::IoEvents;
use crate::fs::device::{Device, DeviceId, DeviceType};
use crate::fs::inode_handle::FileIo;
use crate::prelude::*;
use crate::process::signal::Poller;

pub struct Urandom;

impl Urandom {
    pub fn getrandom(buf: &mut [u8]) -> Result<usize> {
        getrandom::getrandom(buf)?;
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
}

impl FileIo for Urandom {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        Self::getrandom(buf)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        Ok(buf.len())
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}
