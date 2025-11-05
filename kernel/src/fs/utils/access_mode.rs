// SPDX-License-Identifier: MPL-2.0

use aster_rights::Rights;

use crate::prelude::*;

#[expect(non_camel_case_types)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AccessMode {
    /// Read only
    O_RDONLY = 0,
    /// Write only
    O_WRONLY = 1,
    /// Read write
    O_RDWR = 2,
}

impl AccessMode {
    pub fn is_readable(&self) -> bool {
        matches!(*self, AccessMode::O_RDONLY | AccessMode::O_RDWR)
    }

    pub fn is_writable(&self) -> bool {
        matches!(*self, AccessMode::O_WRONLY | AccessMode::O_RDWR)
    }
}

impl AccessMode {
    pub fn from_u32(flags: u32) -> Result<Self> {
        let bits = (flags & 0b11) as u8;
        match bits {
            0 => Ok(Self::O_RDONLY),
            1 => Ok(Self::O_WRONLY),
            2 => Ok(Self::O_RDWR),
            _ => return_errno_with_message!(Errno::EINVAL, "the bits are not a valid access mode"),
        }
    }
}

impl From<AccessMode> for Rights {
    fn from(access_mode: AccessMode) -> Rights {
        match access_mode {
            AccessMode::O_RDONLY => Rights::READ,
            AccessMode::O_WRONLY => Rights::WRITE,
            AccessMode::O_RDWR => Rights::READ | Rights::WRITE,
        }
    }
}
