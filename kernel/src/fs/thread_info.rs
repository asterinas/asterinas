// SPDX-License-Identifier: MPL-2.0

use ostd::sync::{RwLock, RwMutex};

use super::{fs_resolver::FsResolver, utils::FileCreationMask};

/// FS information for a POSIX thread.
pub struct ThreadFsInfo {
    resolver: RwMutex<FsResolver>,
    umask: RwLock<FileCreationMask>,
}

impl ThreadFsInfo {
    /// Creates a new `ThreadFsInfo` with the given [`FsResolver`].
    pub fn new(fs_resolver: FsResolver) -> Self {
        Self {
            resolver: RwMutex::new(fs_resolver),
            umask: RwLock::new(FileCreationMask::default()),
        }
    }

    /// Returns the associated `FsResolver`.
    pub fn resolver(&self) -> &RwMutex<FsResolver> {
        &self.resolver
    }

    /// Returns the associated `FileCreationMask`.
    pub fn umask(&self) -> &RwLock<FileCreationMask> {
        &self.umask
    }
}

impl Clone for ThreadFsInfo {
    fn clone(&self) -> Self {
        Self {
            resolver: RwMutex::new(self.resolver.read().clone()),
            umask: RwLock::new(FileCreationMask::new(self.umask.read().get())),
        }
    }
}
