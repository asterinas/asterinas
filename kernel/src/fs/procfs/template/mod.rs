// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

pub(super) use self::{
    builder::{ProcDirBuilder, ProcFileBuilder, ProcSymBuilder},
    dir::{DirOps, ProcDir, lookup_child_from_table, populate_children_from_table},
    file::{FileOps, ProcFile},
    sym::{ProcSym, SymOps},
};
use super::{BLOCK_SIZE, ProcFs};
use crate::{
    fs::utils::{Extension, FileSystem, InodeMode, InodeType, Metadata},
    prelude::*,
    process::{Gid, Uid},
};

mod builder;
mod dir;
mod file;
mod sym;

struct Common {
    metadata: RwLock<Metadata>,
    extension: Extension,
    fs: Weak<dyn FileSystem>,
    is_volatile: bool,
}

impl Common {
    pub fn new(metadata: Metadata, fs: Weak<dyn FileSystem>, is_volatile: bool) -> Self {
        Self {
            metadata: RwLock::new(metadata),
            extension: Extension::new(),
            fs,
            is_volatile,
        }
    }

    pub fn fs(&self) -> Arc<dyn FileSystem> {
        self.fs.upgrade().unwrap()
    }

    pub fn metadata(&self) -> Metadata {
        let metadata = *self.metadata.read();
        if metadata.container_dev_id.is_null()
            && let Some(fs) = self.fs.upgrade()
        {
            let dev_id = fs.sb().container_dev_id;
            let mut metadata_lock = self.metadata.write();
            metadata_lock.container_dev_id = dev_id;
            return *metadata_lock;
        }
        metadata
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
        self.metadata.read().last_access_at
    }

    pub fn set_atime(&self, time: Duration) {
        self.metadata.write().last_access_at = time;
    }

    pub fn mtime(&self) -> Duration {
        self.metadata.read().last_modify_at
    }

    pub fn set_mtime(&self, time: Duration) {
        self.metadata.write().last_modify_at = time;
    }

    pub fn ctime(&self) -> Duration {
        self.metadata.read().last_meta_change_at
    }

    pub fn set_ctime(&self, time: Duration) {
        self.metadata.write().last_meta_change_at = time;
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

    pub fn extension(&self) -> &Extension {
        &self.extension
    }
}
