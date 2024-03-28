// SPDX-License-Identifier: MPL-2.0

use int_to_c_enum::TryFromInt;

use crate::queue::QueueError;

pub mod block;
pub mod console;
pub mod input;
pub mod network;
pub mod socket;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, TryFromInt)]
#[repr(u8)]
pub enum VirtioDeviceType {
    Invalid = 0,
    Network = 1,
    Block = 2,
    Console = 3,
    Entropy = 4,
    TraditionalMemoryBalloon = 5,
    IoMemory = 6,
    Rpmsg = 7,
    ScsiHost = 8,
    Transport9P = 9,
    Mac80211Wlan = 10,
    RprocSerial = 11,
    VirtioCAIF = 12,
    MemoryBalloon = 13,
    GPU = 16,
    Timer = 17,
    Input = 18,
    Socket = 19,
    Crypto = 20,
    SignalDistribution = 21,
    Pstore = 22,
    IOMMU = 23,
    Memory = 24,
}

#[derive(Debug)]
pub enum VirtioDeviceError {
    /// queues amount do not match the requirement
    /// first element is actual value, second element is expect value
    QueuesAmountDoNotMatch(u16, u16),
    /// unknown error of queue
    QueueUnknownError,
    /// The input virtio capability list contains invalid element
    CapabilityListError,
}

impl From<QueueError> for VirtioDeviceError {
    fn from(_: QueueError) -> Self {
        VirtioDeviceError::QueueUnknownError
    }
}
