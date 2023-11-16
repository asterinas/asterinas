use super::*;
use crate::events::IoEvents;
use crate::fs::inode_handle::FileIo;
use crate::prelude::*;
use crate::process::signal::Poller;

pub struct Zero;

impl Device for Zero {
    fn type_(&self) -> DeviceType {
        DeviceType::CharDevice
    }

    fn id(&self) -> DeviceId {
        // Same value with Linux
        DeviceId::new(1, 5)
    }
}

impl FileIo for Zero {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        for byte in buf.iter_mut() {
            *byte = 0;
        }
        Ok(buf.len())
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        Ok(buf.len())
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}
