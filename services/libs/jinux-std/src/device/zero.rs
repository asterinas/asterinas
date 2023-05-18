use super::*;
use crate::prelude::*;

pub struct Zero;

impl Device for Zero {
    fn type_(&self) -> DeviceType {
        DeviceType::CharDevice
    }

    fn id(&self) -> DeviceId {
        // Same value with Linux
        DeviceId::new(1, 5)
    }

    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        for byte in buf.iter_mut() {
            *byte = 0;
        }
        Ok(buf.len())
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        Ok(buf.len())
    }
}
