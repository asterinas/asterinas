// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::AtomicU16;

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;

use crate::prelude::*;

/// A mask for the file mode of a newly-created file or directory.
///
/// This mask is always a subset of `0o777`.
pub struct FileCreationMask(u16);

impl FileCreationMask {
    /// The valid bits of a `FileCreationMask`.
    const MASK: u16 = 0o777;

    /// Get a new value.
    pub fn get(&self) -> u16 {
        self.0
    }
}

impl Default for FileCreationMask {
    fn default() -> Self {
        Self(0o022)
    }
}

impl TryFrom<u16> for FileCreationMask {
    type Error = Error;

    fn try_from(value: u16) -> Result<Self> {
        if value & !Self::MASK != 0 {
            Err(Error::with_message(
                Errno::EINVAL,
                "Invalid FileCreationMask.",
            ))
        } else {
            Ok(Self(value))
        }
    }
}

impl From<FileCreationMask> for u16 {
    fn from(value: FileCreationMask) -> Self {
        value.0
    }
}

define_atomic_version_of_integer_like_type!(FileCreationMask, try_from = true, {
    /// An atomic version of `FileCreationMask`.
    #[derive(Debug)]
    pub struct AtomicFileCreationMask(AtomicU16);
});
