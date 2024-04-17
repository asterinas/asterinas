// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use aster_frame::sync::Rcu;

pub use self::{
    builder::{ProcDirBuilder, ProcFileBuilder, ProcSymBuilder},
    dir::{DirOps, ProcDir},
    file::FileOps,
    sym::SymOps,
};
use super::ProcFS;
use crate::{
    fs::utils::{FileSystem, InodeMode, Metadata},
    prelude::*,
    process::{Gid, Uid},
};

mod builder;
mod dir;
mod file;
mod sym;

struct Common {
    metadata: Rcu<Box<Metadata>>,
    writer_lock: SpinLock<()>,
    fs: Weak<dyn FileSystem>,
    is_volatile: bool,
}

impl Common {
    pub fn new(metadata: Metadata, fs: Weak<dyn FileSystem>, is_volatile: bool) -> Self {
        Self {
            metadata: Rcu::new(Box::new(metadata)),
            writer_lock: SpinLock::new(()),
            fs,
            is_volatile,
        }
    }

    pub fn fs(&self) -> Arc<dyn FileSystem> {
        self.fs.upgrade().unwrap()
    }

    pub fn metadata(&self) -> Metadata {
        *self.metadata.get()
    }

    pub fn ino(&self) -> u64 {
        self.metadata.get().ino as _
    }

    pub fn size(&self) -> usize {
        self.metadata.get().size
    }

    pub fn atime(&self) -> Duration {
        self.metadata.get().atime
    }

    pub fn set_atime(&self, time: Duration) {
        self.writer_lock.lock();
        let reclaimer = self.metadata.replace({
            let mut metadata = self.metadata.copy();
            metadata.atime = time;
            Box::new(metadata)
        });
        reclaimer.delay();
    }

    pub fn mtime(&self) -> Duration {
        self.metadata.get().mtime
    }

    pub fn set_mtime(&self, time: Duration) {
        self.writer_lock.lock();
        let reclaimer = self.metadata.replace({
            let mut metadata = self.metadata.copy();
            metadata.mtime = time;
            Box::new(metadata)
        });
        reclaimer.delay();
    }

    pub fn mode(&self) -> Result<InodeMode> {
        Ok(self.metadata.get().mode)
    }

    pub fn set_mode(&self, mode: InodeMode) -> Result<()> {
        self.writer_lock.lock();
        let reclaimer = self.metadata.replace({
            let mut metadata = self.metadata.copy();
            metadata.mode = mode;
            Box::new(metadata)
        });
        reclaimer.delay();
        Ok(())
    }

    pub fn owner(&self) -> Result<Uid> {
        Ok(self.metadata.get().uid)
    }

    pub fn set_owner(&self, uid: Uid) -> Result<()> {
        self.writer_lock.lock();
        let reclaimer = self.metadata.replace({
            let mut metadata = self.metadata.copy();
            metadata.uid = uid;
            Box::new(metadata)
        });
        reclaimer.delay();
        Ok(())
    }

    pub fn group(&self) -> Result<Gid> {
        Ok(self.metadata.get().gid)
    }

    pub fn set_group(&self, gid: Gid) -> Result<()> {
        self.writer_lock.lock();
        let reclaimer = self.metadata.replace({
            let mut metadata = self.metadata.copy();
            metadata.gid = gid;
            Box::new(metadata)
        });
        reclaimer.delay();
        Ok(())
    }

    pub fn is_volatile(&self) -> bool {
        self.is_volatile
    }
}
