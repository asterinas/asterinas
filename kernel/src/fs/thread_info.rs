// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use ostd::sync::RwMutex;

use super::{path::PathResolver, utils::AtomicFileCreationMask};
use crate::fs::utils::FileCreationMask;

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
