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
    check_access: bool,
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
            check_access: true,
        })
    }

    /// Creates `OpenArgs` from the given access mode and inode mode.
    pub fn from_modes(access_mode: AccessMode, inode_mode: InodeMode) -> Self {
        Self {
            creation_flags: CreationFlags::empty(),
            status_flags: StatusFlags::empty(),
            access_mode,
            inode_mode,
            check_access: true,
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

    /// Converts the arguments for opening a file that this request has just created.
    ///
    /// Creation-only flags have already served their purpose by this point.
    /// `O_TRUNC` is also cleared
    /// because a newly created file must not be truncated again during open completion.
    pub(crate) fn into_created_file_open(mut self) -> Self {
        self.creation_flags.remove(
            CreationFlags::O_CREAT
                | CreationFlags::O_EXCL
                | CreationFlags::O_TRUNC
                | CreationFlags::O_DIRECTORY
                | CreationFlags::O_TMPFILE,
        );
        self.check_access = false;
        self
    }

    /// Returns whether ordinary inode access checks are required.
    pub(in crate::fs) const fn should_check_access(&self) -> bool {
        self.check_access
    }
}
