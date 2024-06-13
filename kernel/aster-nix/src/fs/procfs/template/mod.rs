// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

pub use self::{
    builder::{ProcDirBuilder, ProcFileBuilder, ProcSymBuilder},
    dir::{DirOps, ProcDir},
    file::FileOps,
    sym::SymOps,
};
use super::{ProcFS, BLOCK_SIZE};
use crate::{
    fs::utils::{FileSystem, InodeMode, InodeType, Metadata},
    prelude::*,
    process::{Gid, Uid},
};

mod builder;
mod dir;
mod file;
mod sym;

struct Common {
    metadata: RwLock<Metadata>,
    fs: Weak<dyn FileSystem>,
    is_volatile: bool,
}

impl Common {
    pub fn new(metadata: Metadata, fs: Weak<dyn FileSystem>, is_volatile: bool) -> Self {
        Self {
            metadata: RwLock::new(metadata),
            fs,
            is_volatile,
        }
    }

    pub fn fs(&self) -> Arc<dyn FileSystem> {
        self.fs.upgrade().unwrap()
    }

    pub fn metadata(&self) -> Metadata {
        *self.metadata.read()
    }

    pub fn ino(&self) -> u64 {
        self.metadata.read().ino
    }

    pub fn type_(&self) -> InodeType {
        self.metadata.read().type_
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

    pub fn ctime(&self) -> Duration {
        self.metadata.read().ctime
    }

    pub fn set_ctime(&self, time: Duration) {
        self.metadata.write().ctime = time;
    }

    pub fn mode(&self) -> Result<InodeMode> {
        Ok(self.metadata.read().mode)
    }

    pub fn set_mode(&self, mode: InodeMode) -> Result<()> {
        self.metadata.write().mode = mode;
        Ok(())
    }

    pub fn owner(&self) -> Result<Uid> {
        Ok(self.metadata.read().uid)
    }

    pub fn set_owner(&self, uid: Uid) -> Result<()> {
        self.metadata.write().uid = uid;
        Ok(())
    }

    pub fn group(&self) -> Result<Gid> {
        Ok(self.metadata.read().gid)
    }

    pub fn set_group(&self, gid: Gid) -> Result<()> {
        self.metadata.write().gid = gid;
        Ok(())
    }

    pub fn is_volatile(&self) -> bool {
        self.is_volatile
    }
}
