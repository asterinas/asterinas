use crate::fs::utils::{Device, DeviceId, DeviceType};
use crate::prelude::*;

pub struct Null;

impl Device for Null {
    fn type_(&self) -> DeviceType {
        DeviceType::CharDevice
    }

    fn id(&self) -> DeviceId {
        // Same value with Linux
        DeviceId::new(1, 3)
    }

    fn read(&self, _buf: &mut [u8]) -> Result<usize> {
        Ok(0)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        Ok(buf.len())
    }
}
