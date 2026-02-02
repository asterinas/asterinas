// SPDX-License-Identifier: MPL-2.0

use crate::{fs::file::AccessMode, prelude::*};

bitflags! {
    pub struct Permission: u16 {
        // This implementation refers the implementation of linux
        // https://elixir.bootlin.com/linux/v6.0.9/source/include/linux/fs.h#L95
        const MAY_EXEC		= 0x0001;
        const MAY_WRITE		= 0x0002;
        const MAY_READ		= 0x0004;
        const MAY_APPEND    = 0x0008;
        const MAY_ACCESS	= 0x0010;
        const MAY_OPEN		= 0x0020;
        const MAY_CHDIR		= 0x0040;
        const MAY_NOT_BLOCK	= 0x0080;
    }
}
impl Permission {
    pub fn may_read(&self) -> bool {
        self.contains(Self::MAY_READ)
    }

    pub fn may_write(&self) -> bool {
        self.contains(Self::MAY_WRITE)
    }

    pub fn may_exec(&self) -> bool {
        self.contains(Self::MAY_EXEC)
    }
}
impl From<AccessMode> for Permission {
    fn from(access_mode: AccessMode) -> Permission {
        match access_mode {
            AccessMode::O_RDONLY => Permission::MAY_READ,
            AccessMode::O_WRONLY => Permission::MAY_WRITE,
            AccessMode::O_RDWR => Permission::MAY_READ | Permission::MAY_WRITE,
        }
    }
}
