// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU16, Ordering};

use atomic_integer_wrapper::define_atomic_version_of_integer_like_type;
use ostd::sync::RwMutex;

use super::vfs::path::PathResolver;
use crate::prelude::*;

/// FS information for a POSIX thread.
pub struct ThreadFsInfo {
    resolver: RwMutex<PathResolver>,
    umask: AtomicFileCreationMask,
}

impl ThreadFsInfo {
    /// Creates a new `ThreadFsInfo` with the given [`PathResolver`].
    pub fn new(path_resolver: PathResolver) -> Self {
        Self {
            resolver: RwMutex::new(path_resolver),
            umask: AtomicFileCreationMask::new(FileCreationMask::default()),
        }
    }

    /// Returns the associated `PathResolver`.
    pub fn resolver(&self) -> &RwMutex<PathResolver> {
        &self.resolver
    }

    /// Returns the associated `FileCreationMask`.
    pub fn umask(&self) -> FileCreationMask {
        self.umask.load(Ordering::Acquire)
    }

    /// Sets a new `FileCreationMask`, returning the old one.
    pub fn swap_umask(&self, new_mask: FileCreationMask) -> FileCreationMask {
        self.umask.swap(new_mask, Ordering::AcqRel)
    }
}

impl Clone for ThreadFsInfo {
    fn clone(&self) -> Self {
        Self {
            resolver: RwMutex::new(self.resolver.read().clone()),
            umask: AtomicFileCreationMask::new(self.umask.load(Ordering::Acquire)),
        }
    }
}

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
