// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use crate::fs::utils::{FileSystem, InodeMode, Metadata};
use crate::prelude::*;

use super::ProcFS;

pub use self::builder::{ProcDirBuilder, ProcFileBuilder, ProcSymBuilder};
pub use self::dir::{DirOps, ProcDir};
pub use self::file::FileOps;
pub use self::sym::SymOps;

mod builder;
mod dir;
mod file;
mod sym;

struct ProcInodeInfo {
    metadata: RwLock<Metadata>,
    fs: Weak<dyn FileSystem>,
    is_volatile: bool,
}

impl ProcInodeInfo {
    pub fn new(metadata: Metadata, fs: Weak<dyn FileSystem>, is_volatile: bool) -> Self {
        Self {
            metadata: RwLock::new(metadata),
            fs,
            is_volatile,
        }
    }

    pub fn fs(&self) -> &Weak<dyn FileSystem> {
        &self.fs
    }

    pub fn metadata(&self) -> Metadata {
        self.metadata.read().clone()
    }

    pub fn ino(&self) -> u64 {
        self.metadata.read().ino as _
    }

    pub fn size(&self) -> usize {
        self.metadata.read().size
    }

    pub fn atime(&self) -> Duration {
        self.metadata.read().atime
    }

    pub fn set_atime(&self, time: Duration) {
        self.metadata.write().atime = time;
    }

    pub fn mtime(&self) -> Duration {
        self.metadata.read().mtime
    }

    pub fn set_mtime(&self, time: Duration) {
        self.metadata.write().mtime = time;
    }

    pub fn mode(&self) -> InodeMode {
        self.metadata.read().mode
    }

    pub fn set_mode(&self, mode: InodeMode) {
        self.metadata.write().mode = mode;
    }

    pub fn is_volatile(&self) -> bool {
        self.is_volatile
    }
}
