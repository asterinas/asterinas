// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use ostd::sync::RwMutex;

use super::{fs_resolver::FsResolver, utils::AtomicFileCreationMask};
use crate::fs::utils::FileCreationMask;

/// FS information for a POSIX thread.
pub struct ThreadFsInfo {
    resolver: RwMutex<FsResolver>,
    umask: AtomicFileCreationMask,
}

impl ThreadFsInfo {
    /// Creates a new `ThreadFsInfo` with the given [`FsResolver`].
    pub fn new(fs_resolver: FsResolver) -> Self {
        Self {
            resolver: RwMutex::new(fs_resolver),
            umask: AtomicFileCreationMask::new(FileCreationMask::default()),
        }
    }

    /// Returns the associated `FsResolver`.
    pub fn resolver(&self) -> &RwMutex<FsResolver> {
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
