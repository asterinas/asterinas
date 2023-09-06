use crate::io_events::IoEvents;
use crate::ioctl::IoctlCmd;
use crate::poll::Poller;
use crate::prelude::*;

/// The abstract of device
pub trait Device: Sync + Send {
    /// Return the device type.
    fn type_(&self) -> DeviceType;

    /// Return the device ID.
    fn id(&self) -> DeviceId;

    /// Read from the device.
    fn read(&self, buf: &mut [u8]) -> Result<usize>;

    /// Write to the device.
    fn write(&self, buf: &[u8]) -> Result<usize>;

    /// Poll on the device.
    fn poll(&self, mask: IoEvents, _poller: Option<&Poller>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }

    /// Ioctl on the device.
    fn ioctl(&self, _cmd: IoctlCmd, _arg: usize) -> Result<i32> {
        Err(Error::new(Errno::EINVAL))
    }
}

impl Debug for dyn Device {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("Device")
            .field("type", &self.type_())
            .field("id", &self.id())
            .finish()
    }
}

/// Device type
#[derive(Debug)]
pub enum DeviceType {
    CharDevice,
    BlockDevice,
}

/// Device Id
#[derive(Clone, Copy)]
pub struct DeviceId(u64);

impl DeviceId {
    pub fn new(major: u32, minor: u32) -> Self {
        let major = major as u64;
        let minor = minor as u64;
        Self(
            (major & 0xffff_f000) << 32
                | (major & 0x0000_0fff) << 8
                | (minor & 0xffff_ff00) << 12
                | (minor & 0x0000_00ff),
        )
    }

    pub fn major(&self) -> u32 {
        ((self.0 >> 32) & 0xffff_f000 | (self.0 >> 8) & 0x0000_0fff) as u32
    }

    pub fn minor(&self) -> u32 {
        ((self.0 >> 12) & 0xffff_ff00 | self.0 & 0x0000_00ff) as u32
    }
}

impl Debug for DeviceId {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("DeviceId")
            .field("major", &self.major())
            .field("minor", &self.minor())
            .finish()
    }
}

impl From<DeviceId> for u64 {
    fn from(value: DeviceId) -> Self {
        value.0
    }
}
