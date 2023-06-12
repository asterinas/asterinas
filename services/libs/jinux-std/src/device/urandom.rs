use crate::fs::device::{Device, DeviceId, DeviceType};
use crate::prelude::*;

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

    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        Self::getrandom(buf)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        Ok(buf.len())
    }
}
