use crate::fs::device::{Device, DeviceId, DeviceType};
use crate::prelude::*;

pub struct Random;

impl Random {
    pub fn getrandom(buf: &mut [u8]) -> Result<usize> {
        getrandom::getrandom(buf)?;
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

    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        Self::getrandom(buf)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        Ok(buf.len())
    }
}

impl From<getrandom::Error> for Error {
    fn from(value: getrandom::Error) -> Self {
        Error::with_message(Errno::ENOSYS, "cannot generate random bytes")
    }
}
