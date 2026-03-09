// SPDX-License-Identifier: MPL-2.0

use int_to_c_enum::TryFromInt;

use crate::{device::DeviceType, prelude::*};

#[repr(u16)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, TryFromInt)]
pub enum InodeType {
    Unknown = 0o000000,
    NamedPipe = 0o010000,
    CharDevice = 0o020000,
    Dir = 0o040000,
    BlockDevice = 0o060000,
    File = 0o100000,
    SymLink = 0o120000,
    Socket = 0o140000,
}

impl InodeType {
    pub fn is_regular_file(&self) -> bool {
        *self == InodeType::File
    }

    pub fn is_directory(&self) -> bool {
        *self == InodeType::Dir
    }

    pub fn is_device(&self) -> bool {
        *self == InodeType::BlockDevice || *self == InodeType::CharDevice
    }

    pub fn is_seekable(&self) -> bool {
        *self != InodeType::NamedPipe && *self != Self::Socket
    }

    /// Parse the inode type in the `mode` from syscall, and convert it into `InodeType`.
    pub fn from_raw_mode(mut mode: u16) -> Result<Self> {
        const TYPE_MASK: u16 = 0o170000;
        mode &= TYPE_MASK;

        // Special case
        if mode == 0 {
            return Ok(Self::File);
        }
        Self::try_from(mode & TYPE_MASK)
            .map_err(|_| Error::with_message(Errno::EINVAL, "invalid file type"))
    }

    pub fn device_type(&self) -> Option<DeviceType> {
        match self {
            InodeType::BlockDevice => Some(DeviceType::Block),
            InodeType::CharDevice => Some(DeviceType::Char),
            _ => None,
        }
    }
}

impl From<DeviceType> for InodeType {
    fn from(type_: DeviceType) -> InodeType {
        match type_ {
            DeviceType::Char => InodeType::CharDevice,
            DeviceType::Block => InodeType::BlockDevice,
        }
    }
}
