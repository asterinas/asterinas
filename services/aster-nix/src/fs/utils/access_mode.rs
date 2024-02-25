// SPDX-License-Identifier: MPL-2.0

use aster_rights::Rights;

use crate::prelude::*;

#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum AccessMode {
    /// read only
    O_RDONLY = 0,
    /// write only
    O_WRONLY = 1,
    /// read write
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
        if bits > Self::O_RDWR as u8 {
            return_errno_with_message!(Errno::EINVAL, "invalid bits for access mode");
        }
        Ok(match bits {
            0 => Self::O_RDONLY,
            1 => Self::O_WRONLY,
            2 => Self::O_RDWR,
            _ => unreachable!(),
        })
    }
}

impl From<Rights> for AccessMode {
    fn from(rights: Rights) -> AccessMode {
        if rights.contains(Rights::READ) && rights.contains(Rights::WRITE) {
            AccessMode::O_RDWR
        } else if rights.contains(Rights::READ) {
            AccessMode::O_RDONLY
        } else if rights.contains(Rights::WRITE) {
            AccessMode::O_WRONLY
        } else {
            panic!("invalid rights");
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
