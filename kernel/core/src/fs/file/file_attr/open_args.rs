// SPDX-License-Identifier: MPL-2.0

use crate::{
    fs::file::{AccessMode, CreationFlags, InodeMode, StatusFlags},
    prelude::*,
};

/// Arguments for an open request.
#[derive(Debug)]
pub struct OpenArgs {
    pub creation_flags: CreationFlags,
    pub status_flags: StatusFlags,
    pub access_mode: AccessMode,
    pub inode_mode: InodeMode,
}

impl OpenArgs {
    /// Creates `OpenArgs` from the given flags and mode.
    pub fn from_flags_and_mode(flags: u32, inode_mode: InodeMode) -> Result<Self> {
        let creation_flags = CreationFlags::from_bits_truncate(flags);
        let status_flags = StatusFlags::from_bits_truncate(flags);
        let access_mode = AccessMode::from_u32(flags)?;

        // When `O_PATH` is set, all other flags (including `O_TMPFILE`) are
        // ignored, so the `O_TMPFILE` validations are skipped.
        // Reference: <https://man7.org/linux/man-pages/man2/open.2.html>.
        if creation_flags.contains(CreationFlags::O_TMPFILE)
            && !status_flags.contains(StatusFlags::O_PATH)
        {
            if !creation_flags.contains(CreationFlags::O_DIRECTORY) {
                return_errno_with_message!(Errno::EINVAL, "O_TMPFILE requires O_DIRECTORY");
            }
            if !access_mode.is_writable() {
                return_errno_with_message!(Errno::EINVAL, "O_TMPFILE requires O_RDWR or O_WRONLY");
            }
            if creation_flags.contains(CreationFlags::O_CREAT) {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "O_TMPFILE and O_CREAT are mutually exclusive"
                );
            }
        } else if creation_flags.contains(CreationFlags::O_CREAT)
            && creation_flags.contains(CreationFlags::O_DIRECTORY)
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "O_CREAT and O_DIRECTORY cannot be specified together"
            );
        }

        Ok(Self {
            creation_flags,
            status_flags,
            access_mode,
            inode_mode,
        })
    }

    /// Creates `OpenArgs` from the given access mode and inode mode.
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

    /// Returns whether this is an `O_TMPFILE` open request.
    pub fn is_tmpfile(&self) -> bool {
        self.creation_flags.contains(CreationFlags::O_TMPFILE)
            && !self.status_flags.contains(StatusFlags::O_PATH)
    }
}
