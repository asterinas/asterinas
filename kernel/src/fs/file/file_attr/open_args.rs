// SPDX-License-Identifier: MPL-2.0

use super::{AccessMode, CreationFlags, StatusFlags};
use crate::{fs::file::InodeMode, prelude::*};

/// Arguments for an open request.
#[derive(Debug)]
pub struct OpenArgs {
    pub creation_flags: CreationFlags,
    pub status_flags: StatusFlags,
    pub access_mode: AccessMode,
    pub inode_mode: InodeMode,
}

impl OpenArgs {
    /// Create `OpenArgs` from the given flags and mode.
    pub fn from_flags_and_mode(flags: u32, inode_mode: InodeMode) -> Result<Self> {
        let creation_flags = CreationFlags::from_bits_truncate(flags);
        let status_flags = StatusFlags::from_bits_truncate(flags);
        let access_mode = AccessMode::from_u32(flags)?;
        Ok(Self {
            creation_flags,
            status_flags,
            access_mode,
            inode_mode,
        })
    }

    /// Create `OpenArgs` from the given access mode and inode mode.
    pub fn from_modes(access_mode: AccessMode, inode_mode: InodeMode) -> Self {
        Self {
            creation_flags: CreationFlags::empty(),
            status_flags: StatusFlags::empty(),
            access_mode,
            inode_mode,
        }
    }

    /// Returns whether to follow the tail link when resolving the path.
    pub fn follow_tail_link(&self) -> bool {
        !(self.creation_flags.contains(CreationFlags::O_NOFOLLOW)
            || self.creation_flags.contains(CreationFlags::O_CREAT)
                && self.creation_flags.contains(CreationFlags::O_EXCL))
    }
}
