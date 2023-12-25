use super::*;
use crate::events::IoEvents;
use crate::fs::inode_handle::FileIo;
use crate::prelude::*;
use crate::process::signal::Poller;

pub struct Null;

impl Device for Null {
    fn type_(&self) -> DeviceType {
        DeviceType::CharDevice
    }

    fn id(&self) -> DeviceId {
        // Same value with Linux
        DeviceId::new(1, 3)
    }
}

impl FileIo for Null {
    fn read(&self, _buf: &mut [u8]) -> Result<usize> {
        Ok(0)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        Ok(buf.len())
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}
